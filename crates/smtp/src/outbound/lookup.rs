/*
 * Copyright (c) 2023 Stalwart Labs Ltd.
 *
 * This file is part of Stalwart Mail Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::{net::IpAddr, sync::Arc};

use mail_auth::{IpLookupStrategy, MX};
use rand::{seq::SliceRandom, Rng};
use utils::config::KeyLookup;

use crate::{
    config::EnvelopeKey,
    core::SMTP,
    queue::{Error, ErrorDetails, Status},
};

use super::NextHop;

pub struct IpLookupResult {
    pub source_ipv4: Option<IpAddr>,
    pub source_ipv6: Option<IpAddr>,
    pub remote_ips: Vec<IpAddr>,
}

impl SMTP {
    pub async fn ip_lookup(
        &self,
        key: &str,
        strategy: IpLookupStrategy,
        max_results: usize,
    ) -> mail_auth::Result<Vec<IpAddr>> {
        let (has_ipv4, has_ipv6, v4_first) = match strategy {
            IpLookupStrategy::Ipv4Only => (true, false, false),
            IpLookupStrategy::Ipv6Only => (false, true, false),
            IpLookupStrategy::Ipv4thenIpv6 => (true, true, true),
            IpLookupStrategy::Ipv6thenIpv4 => (true, true, false),
        };
        let ipv4_addrs = if has_ipv4 {
            match self.resolvers.dns.ipv4_lookup(key).await {
                Ok(addrs) => addrs,
                Err(_) if has_ipv6 => Arc::new(Vec::new()),
                Err(err) => return Err(err),
            }
        } else {
            Arc::new(Vec::new())
        };

        if has_ipv6 {
            let ipv6_addrs = match self.resolvers.dns.ipv6_lookup(key).await {
                Ok(addrs) => addrs,
                Err(_) if !ipv4_addrs.is_empty() => Arc::new(Vec::new()),
                Err(err) => return Err(err),
            };
            if v4_first {
                Ok(ipv4_addrs
                    .iter()
                    .copied()
                    .map(IpAddr::from)
                    .chain(ipv6_addrs.iter().copied().map(IpAddr::from))
                    .take(max_results)
                    .collect())
            } else {
                Ok(ipv6_addrs
                    .iter()
                    .copied()
                    .map(IpAddr::from)
                    .chain(ipv4_addrs.iter().copied().map(IpAddr::from))
                    .take(max_results)
                    .collect())
            }
        } else {
            Ok(ipv4_addrs
                .iter()
                .take(max_results)
                .copied()
                .map(IpAddr::from)
                .collect())
        }
    }

    pub async fn resolve_host(
        &self,
        remote_host: &NextHop<'_>,
        envelope: &impl KeyLookup<Key = EnvelopeKey>,
        max_multihomed: usize,
    ) -> Result<IpLookupResult, Status<(), Error>> {
        let remote_ips = self
            .ip_lookup(
                remote_host.fqdn_hostname().as_ref(),
                *self.queue.config.ip_strategy.eval(envelope).await,
                max_multihomed,
            )
            .await
            .map_err(|err| {
                if let mail_auth::Error::DnsRecordNotFound(_) = &err {
                    Status::PermanentFailure(Error::ConnectionError(ErrorDetails {
                        entity: remote_host.hostname().to_string(),
                        details: "record not found for MX".to_string(),
                    }))
                } else {
                    Status::TemporaryFailure(Error::ConnectionError(ErrorDetails {
                        entity: remote_host.hostname().to_string(),
                        details: format!("lookup error: {err}"),
                    }))
                }
            })?;

        if !remote_ips.is_empty() {
            let mut result = IpLookupResult {
                source_ipv4: None,
                source_ipv6: None,
                remote_ips,
            };

            // Obtain source IPv4 address
            let source_ips = self.queue.config.source_ip.ipv4.eval(envelope).await;
            match source_ips.len().cmp(&1) {
                std::cmp::Ordering::Equal => {
                    result.source_ipv4 = IpAddr::from(*source_ips.first().unwrap()).into();
                }
                std::cmp::Ordering::Greater => {
                    result.source_ipv4 =
                        IpAddr::from(source_ips[rand::thread_rng().gen_range(0..source_ips.len())])
                            .into();
                }
                std::cmp::Ordering::Less => (),
            }

            // Obtain source IPv6 address
            let source_ips = self.queue.config.source_ip.ipv6.eval(envelope).await;
            match source_ips.len().cmp(&1) {
                std::cmp::Ordering::Equal => {
                    result.source_ipv6 = IpAddr::from(*source_ips.first().unwrap()).into();
                }
                std::cmp::Ordering::Greater => {
                    result.source_ipv6 =
                        IpAddr::from(source_ips[rand::thread_rng().gen_range(0..source_ips.len())])
                            .into();
                }
                std::cmp::Ordering::Less => (),
            }

            Ok(result)
        } else {
            Err(Status::TemporaryFailure(Error::DnsError(format!(
                "No IP addresses found for {:?}.",
                envelope.key(&EnvelopeKey::Mx)
            ))))
        }
    }
}

pub trait ToNextHop {
    fn to_remote_hosts<'x, 'y: 'x>(
        &'x self,
        domain: &'y str,
        max_mx: usize,
    ) -> Option<Vec<NextHop<'_>>>;
}

impl ToNextHop for Vec<MX> {
    fn to_remote_hosts<'x, 'y: 'x>(
        &'x self,
        domain: &'y str,
        max_mx: usize,
    ) -> Option<Vec<NextHop<'_>>> {
        if !self.is_empty() {
            // Obtain max number of MX hosts to process
            let mut remote_hosts = Vec::with_capacity(max_mx);

            'outer: for mx in self.iter() {
                if mx.exchanges.len() > 1 {
                    let mut slice = mx.exchanges.iter().collect::<Vec<_>>();
                    slice.shuffle(&mut rand::thread_rng());
                    for remote_host in slice {
                        remote_hosts.push(NextHop::MX(remote_host.as_str()));
                        if remote_hosts.len() == max_mx {
                            break 'outer;
                        }
                    }
                } else if let Some(remote_host) = mx.exchanges.first() {
                    // Check for Null MX
                    if mx.preference == 0 && remote_host == "." {
                        return None;
                    }
                    remote_hosts.push(NextHop::MX(remote_host.as_str()));
                    if remote_hosts.len() == max_mx {
                        break;
                    }
                }
            }
            remote_hosts.into()
        } else {
            // If an empty list of MXs is returned, the address is treated as if it was
            // associated with an implicit MX RR with a preference of 0, pointing to that host.
            vec![NextHop::MX(domain)].into()
        }
    }
}
