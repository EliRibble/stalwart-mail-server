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

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Instant,
};

use directory::QueryBy;
use hyper::header;
use jmap_proto::error::request::RequestError;
use mail_parser::decoders::base64::base64_decode;
use mail_send::Credentials;
use utils::{listener::limiter::InFlight, map::ttl_dashmap::TtlMap};

use crate::JMAP;

use super::{rate_limit::RemoteAddress, AccessToken};

impl JMAP {
    pub async fn authenticate_headers(
        &self,
        req: &hyper::Request<hyper::body::Incoming>,
        remote_ip: IpAddr,
    ) -> Result<Option<(InFlight, Arc<AccessToken>)>, RequestError> {
        if let Some((mechanism, token)) = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split_once(' ').map(|(l, t)| (l, t.trim().to_string())))
        {
            let session = if let Some(account_id) = self.sessions.get_with_ttl(&token) {
                self.get_cached_access_token(account_id).await
            } else {
                let addr = self.build_remote_addr(req, remote_ip);
                if mechanism.eq_ignore_ascii_case("basic") {
                    // Enforce rate limit for authentication requests
                    self.is_auth_allowed_soft(&addr)?;

                    // Decode the base64 encoded credentials
                    if let Some((account, secret)) = base64_decode(token.as_bytes())
                        .and_then(|token| String::from_utf8(token).ok())
                        .and_then(|token| {
                            token.split_once(':').map(|(login, secret)| {
                                (login.trim().to_lowercase(), secret.to_string())
                            })
                        })
                    {
                        self.authenticate_plain(&account, &secret, &addr).await
                    } else {
                        tracing::debug!(
                            context = "authenticate_headers",
                            token = token,
                            "Failed to decode Basic auth request.",
                        );
                        None
                    }
                } else if mechanism.eq_ignore_ascii_case("bearer") {
                    // Enforce anonymous rate limit for bearer auth requests
                    self.is_anonymous_allowed(&addr)?;

                    match self.validate_access_token("access_token", &token).await {
                        Ok((account_id, _, _)) => self.get_access_token(account_id).await,
                        Err(err) => {
                            tracing::debug!(
                                context = "authenticate_headers",
                                err = err,
                                "Failed to validate access token."
                            );
                            None
                        }
                    }
                } else {
                    // Enforce anonymous rate limit
                    self.is_anonymous_allowed(&addr)?;
                    None
                }
                .map(|access_token| {
                    let access_token = Arc::new(access_token);
                    self.cache_session(token, &access_token);
                    self.cache_access_token(access_token.clone());
                    access_token
                })
            };

            if let Some(session) = session {
                // Enforce authenticated rate limit
                Ok(Some((self.is_account_allowed(&session)?, session)))
            } else {
                Ok(None)
            }
        } else {
            // Enforce anonymous rate limit
            self.is_anonymous_allowed(&self.build_remote_addr(req, remote_ip))?;

            Ok(None)
        }
    }

    pub fn cache_session(&self, session_id: String, access_token: &AccessToken) {
        self.sessions.insert_with_ttl(
            session_id,
            access_token.primary_id(),
            Instant::now() + self.config.session_cache_ttl,
        );
    }

    pub fn cache_access_token(&self, access_token: Arc<AccessToken>) {
        self.access_tokens.insert_with_ttl(
            access_token.primary_id(),
            access_token,
            Instant::now() + self.config.session_cache_ttl,
        );
    }

    pub async fn get_cached_access_token(&self, primary_id: u32) -> Option<Arc<AccessToken>> {
        if let Some(access_token) = self.access_tokens.get_with_ttl(&primary_id) {
            access_token.into()
        } else {
            // Refresh ACL token
            self.get_access_token(primary_id).await.map(|access_token| {
                let access_token = Arc::new(access_token);
                self.cache_access_token(access_token.clone());
                access_token
            })
        }
    }

    pub fn build_remote_addr(
        &self,
        req: &hyper::Request<hyper::body::Incoming>,
        remote_ip: IpAddr,
    ) -> RemoteAddress {
        if !self.config.rate_use_forwarded {
            RemoteAddress::IpAddress(remote_ip)
        } else if let Some(forwarded_for) = req
            .headers()
            .get(header::FORWARDED)
            .and_then(|h| h.to_str().ok())
        {
            RemoteAddress::IpAddressFwd(forwarded_for.trim().to_string())
        } else {
            tracing::debug!("Warning: No remote address found in request, using loopback.");
            RemoteAddress::IpAddress(Ipv4Addr::new(127, 0, 0, 1).into())
        }
    }

    pub async fn authenticate_plain(
        &self,
        username: &str,
        secret: &str,
        remote_addr: &RemoteAddress,
    ) -> Option<AccessToken> {
        match self
            .directory
            .query(
                QueryBy::Credentials(&Credentials::Plain {
                    username: username.to_string(),
                    secret: secret.to_string(),
                }),
                true,
            )
            .await
        {
            Ok(Some(principal)) => AccessToken::new(principal).into(),
            Ok(None) => {
                let _ = self.is_auth_allowed_hard(remote_addr);
                None
            }
            Err(_) => None,
        }
    }

    pub async fn get_access_token(&self, account_id: u32) -> Option<AccessToken> {
        // Create access token
        self.update_access_token(AccessToken::new(
            self.directory
                .query(QueryBy::Id(account_id), true)
                .await
                .ok()??,
        ))
        .await
    }
}
