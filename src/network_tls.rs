//! Configure reqwest from native Git HTTP options without a process-global TLS provider.

use gix::bstr::ByteSlice;
use std::sync::{Arc, Mutex};

use gix::protocol::transport::client::blocking_io::{Transport, http};

fn invalid(message: impl Into<String>) -> gix::Error {
    gix::Error::from_error(std::io::Error::other(message.into()))
}

pub fn configure<T: Transport>(connection: &mut gix::remote::Connection<'_, '_, '_, T>) -> gix::Result<()> {
    let url = connection.transport_mut().to_url().into_owned();
    let remote = connection.remote();
    let Some(mut options) = remote
        .repo()
        .transport_options(url.as_bstr(), remote.name().map(|n| n.as_bstr()))?
    else {
        return Ok(());
    };
    if let Some(http_options) = options.downcast_mut::<http::Options>() {
        let settings = http_options.clone();
        http_options.backend = Some(Arc::new(Mutex::new(http::reqwest::Options {
            configure_client: Some(Box::new(move |builder| configure_builder(builder, &settings))),
            ..Default::default()
        })));
    }
    connection.set_transport_options(options);
    Ok(())
}

fn configure_builder(
    mut builder: reqwest::blocking::ClientBuilder,
    options: &http::Options,
) -> gix::Result<reqwest::blocking::ClientBuilder> {
    if let Some(timeout) = options.connect_timeout {
        builder = builder.connect_timeout(timeout);
    }
    if let Some(agent) = &options.user_agent {
        builder = builder.user_agent(agent);
    }
    builder = builder.connection_verbose(options.verbose);
    if let Some(proxy) = &options.proxy {
        if proxy.is_empty() {
            builder = builder.no_proxy();
        } else {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(gix::Error::from_error)?
                .no_proxy(options.no_proxy.as_deref().and_then(reqwest::NoProxy::from_string));
            builder = builder.proxy(proxy);
        }
    }
    if options.no_proxy.as_deref() == Some("*") {
        builder = builder.no_proxy();
    }
    if let Some(http::options::HttpVersion::V1_1) = options.http_version {
        builder = builder.http1_only();
    }
    if options.low_speed_limit_bytes_per_second != 0 && options.low_speed_time_seconds != 0 {
        return Err(invalid("reqwest does not support Git's HTTP low-speed limit"));
    }
    if options.proxy_authenticate.is_some() {
        return Err(invalid("reqwest does not support Git proxy credential helpers"));
    }
    #[cfg(feature = "https")]
    {
        builder = builder.use_preconfigured_tls(tls_config(options)?);
    }
    Ok(builder)
}

#[cfg(feature = "https")]
fn crypto_provider() -> gix::Result<Arc<rustls::crypto::CryptoProvider>> {
    // These are Graviola 0.4.1's mandatory requirements, checked before any provider operation.
    #[cfg(target_arch = "x86_64")]
    let supported = std::is_x86_feature_detected!("aes")
        && std::is_x86_feature_detected!("pclmulqdq")
        && std::is_x86_feature_detected!("bmi1")
        && std::is_x86_feature_detected!("adx")
        && std::is_x86_feature_detected!("avx")
        && std::is_x86_feature_detected!("avx2");
    #[cfg(target_arch = "aarch64")]
    let supported = std::arch::is_aarch64_feature_detected!("neon")
        && std::arch::is_aarch64_feature_detected!("aes")
        && std::arch::is_aarch64_feature_detected!("pmull")
        && std::arch::is_aarch64_feature_detected!("sha2");
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let supported = false;
    if !supported {
        return Err(invalid(
            "this CPU does not support the instructions required by the Graviola TLS provider",
        ));
    }
    Ok(Arc::new(rustls_graviola::default_provider()))
}

#[cfg(feature = "https")]
fn tls_config(options: &http::Options) -> gix::Result<rustls::ClientConfig> {
    use http::options::SslVersion;
    use rustls::pki_types::{CertificateDer, pem::PemObject};
    use rustls_platform_verifier::BuilderVerifierExt;

    let provider = crypto_provider()?;
    let mut versions = vec![&rustls::version::TLS13, &rustls::version::TLS12];
    if let Some(range) = options.ssl_version {
        let (min, max) = range.min_max();
        let version = |value| match value {
            SslVersion::Default => None,
            SslVersion::TlsV1_3 => Some(13),
            SslVersion::TlsV1_2 => Some(12),
            SslVersion::TlsV1_1 => Some(11),
            SslVersion::TlsV1 | SslVersion::TlsV1_0 => Some(10),
            SslVersion::SslV2 => Some(2),
            SslVersion::SslV3 => Some(3),
        };
        versions.retain(|v| {
            let number = if v.version == rustls::ProtocolVersion::TLSv1_3 {
                13
            } else {
                12
            };
            version(min).is_none_or(|min| number >= min) && version(max).is_none_or(|max| number <= max)
        });
    }
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&versions)
        .map_err(gix::Error::from_error)?;
    let builder = if !options.ssl_verify {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(UnverifiedCertificates(provider)))
    } else if let Some(path) = &options.ssl_ca_info {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_file_iter(path).map_err(gix::Error::from_error)? {
            roots
                .add(cert.map_err(gix::Error::from_error)?)
                .map_err(gix::Error::from_error)?;
        }
        if roots.is_empty() {
            return Err(invalid("http.sslCAInfo contains no CA certificates"));
        }
        builder.with_root_certificates(roots)
    } else {
        builder.with_platform_verifier().map_err(gix::Error::from_error)?
    };
    let mut config = builder.with_no_client_auth();
    config.alpn_protocols = match options.http_version {
        Some(http::options::HttpVersion::V1_1) => vec![b"http/1.1".to_vec()],
        _ => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    };
    Ok(config)
}

#[cfg(feature = "https")]
#[derive(Debug)]
struct UnverifiedCertificates(Arc<rustls::crypto::CryptoProvider>);

#[cfg(feature = "https")]
impl rustls::client::danger::ServerCertVerifier for UnverifiedCertificates {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, signature, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, signature, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
