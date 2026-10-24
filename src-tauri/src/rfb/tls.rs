//! TLS for VeNCrypt X.509 subtypes.
//!
//! A certificate is accepted if a known authority signed it for the host name,
//! or if its SHA-256 fingerprint is the one the user chose to trust (VNC servers
//! mostly use self-signed certificates). Otherwise the connection fails with
//! [`Error::UntrustedCertificate`] carrying the fingerprint, so the UI can ask.

use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use super::{BoxStream, Error, Result};

/// SHA-256 of a DER certificate, as colon-separated uppercase hex pairs.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub async fn connect(
    stream: BoxStream,
    host: &str,
    trusted: Option<&str>,
) -> Result<TlsStream<BoxStream>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let webpki = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| Error::Tls(e.to_string()))?;
    let rejected = Arc::new(Mutex::new(None));
    let verifier = Arc::new(Verifier {
        webpki,
        trusted: trusted.map(str::to_ascii_uppercase),
        rejected: rejected.clone(),
        provider: provider.clone(),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned())
        .or_else(|_| ServerName::try_from("vnc.invalid"))
        .expect("valid fallback name");
    TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|e| match rejected.lock().unwrap().take() {
            Some(fp) => Error::UntrustedCertificate(fp),
            None => Error::Tls(e.to_string()),
        })
}

#[derive(Debug)]
struct Verifier {
    webpki: Arc<WebPkiServerVerifier>,
    trusted: Option<String>,
    /// Fingerprint of a certificate that failed verification.
    rejected: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity);
        if self.trusted.as_deref() == Some(fp.as_str()) {
            return Ok(ServerCertVerified::assertion());
        }
        self.webpki
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            .inspect_err(|_| *self.rejected.lock().unwrap() = Some(fp))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
