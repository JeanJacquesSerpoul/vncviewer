//! Parsing of the server field, which accepts the usual VNC notations.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    Empty,
    InvalidPort(String),
    BadIpv6,
}

/// Resolves `host` and the port field into a host and a TCP port:
///
/// * `host` alone uses `default_port`;
/// * `host:N` is a display number when N < 100 (port 5900 + N), a port otherwise;
/// * `host::PORT` is always a port;
/// * IPv6 addresses go in brackets (`[v6addr]:N`); a bare address is accepted
///   when it can't be read as `host::port` (as with TigerVNC).
pub fn resolve(host: &str, default_port: u16) -> Result<(String, u16), AddressError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(AddressError::Empty);
    }
    let parse = |s: &str| {
        s.parse::<u16>()
            .map_err(|_| AddressError::InvalidPort(s.into()))
    };

    if let Some(rest) = host.strip_prefix('[') {
        let (addr, tail) = rest.split_once(']').ok_or(AddressError::BadIpv6)?;
        return match tail.strip_prefix(':') {
            Some(p) => Ok((addr.into(), display_or_port(parse(p)?))),
            None if tail.is_empty() => Ok((addr.into(), default_port)),
            None => Err(AddressError::BadIpv6),
        };
    }
    if let Some((name, port)) = host.split_once("::")
        && !name.contains(':')
        && !port.contains(':')
        && !name.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
        && !port.is_empty()
    {
        return Ok((name.into(), parse(port)?));
    }
    match host.matches(':').count() {
        0 => Ok((host.into(), default_port)),
        1 => {
            let (name, n) = host.split_once(':').unwrap();
            if name.is_empty() {
                return Err(AddressError::Empty);
            }
            Ok((name.into(), display_or_port(parse(n)?)))
        }
        // Bare IPv6 address.
        _ => Ok((host.into(), default_port)),
    }
}

fn display_or_port(n: u16) -> u16 {
    if n < 100 { 5900 + n } else { n }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notations() {
        assert_eq!(resolve("pc", 5900), Ok(("pc".into(), 5900)));
        assert_eq!(resolve(" pc:1 ", 5900), Ok(("pc".into(), 5901)));
        assert_eq!(resolve("pc:5905", 5900), Ok(("pc".into(), 5905)));
        assert_eq!(resolve("pc::22", 5900), Ok(("pc".into(), 22)));
        assert_eq!(resolve("[::1]:2", 5900), Ok(("::1".into(), 5902)));
        assert_eq!(resolve("fe80::1:2", 5900), Ok(("fe80::1:2".into(), 5900)));
        assert_eq!(resolve("[fe80::1]", 5900), Ok(("fe80::1".into(), 5900)));
        assert_eq!(
            resolve("pc:x", 5900),
            Err(AddressError::InvalidPort("x".into()))
        );
        assert_eq!(resolve("[::1", 5900), Err(AddressError::BadIpv6));
        assert_eq!(resolve(" ", 5900), Err(AddressError::Empty));
    }
}
