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

use std::{io::Cursor, sync::Arc};

use rustls::{
    server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni},
    sign::CertifiedKey,
    version::{TLS12, TLS13},
    SupportedProtocolVersion,
};
use rustls_pemfile::{certs, read_one, Item};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

use super::Config;

pub static TLS13_VERSION: &[&SupportedProtocolVersion] = &[&TLS13];
pub static TLS12_VERSION: &[&SupportedProtocolVersion] = &[&TLS12];

#[derive(Debug)]
pub struct CertificateResolver {
    pub resolver: Option<ResolvesServerCertUsingSni>,
    pub default_cert: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for CertificateResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.resolver
            .as_ref()
            .and_then(|r| r.resolve(hello))
            .or_else(|| self.default_cert.clone())
    }
}

impl Config {
    pub fn rustls_certificate(&self, cert_id: &str) -> super::Result<Vec<CertificateDer<'static>>> {
        let certs = certs(&mut Cursor::new(self.file_contents((
            "certificate",
            cert_id,
            "cert",
        ))?))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| {
            format!("Failed to read certificates in \"certificate.{cert_id}.cert\": {err}")
        })?;

        if !certs.is_empty() {
            Ok(certs)
        } else {
            Err(format!(
                "No certificates found in \"certificate.{cert_id}.cert\"."
            ))
        }
    }

    pub fn rustls_private_key(&self, cert_id: &str) -> super::Result<PrivateKeyDer<'static>> {
        match read_one(&mut Cursor::new(self.file_contents((
            "certificate",
            cert_id,
            "private-key",
        ))?))
        .map_err(|err| {
            format!("Failed to read private keys in \"certificate.{cert_id}.private-key\": {err}",)
        })?
        .into_iter()
        .next()
        {
            Some(Item::Pkcs8Key(key)) => Ok(PrivateKeyDer::Pkcs8(key)),
            Some(Item::Pkcs1Key(key)) => Ok(PrivateKeyDer::Pkcs1(key)),
            Some(Item::Sec1Key(key)) => Ok(PrivateKeyDer::Sec1(key)),
            Some(_) => Err(format!(
                "Unsupported private keys found in \"certificate.{cert_id}.private-key\".",
            )),
            None => Err(format!(
                "No private keys found in \"certificate.{cert_id}.private-key\".",
            )),
        }
    }
}
