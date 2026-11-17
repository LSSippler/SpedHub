//! Versionsprüfung zwischen Server und Launcher.
//!
//! Server und Launcher passen zusammen, wenn Haupt- und Nebenversion gleich sind: `0.2.0` passt
//! zu `0.2.7`, aber nicht zu `0.3.0`. Der Launcher schickt seine Version im Header
//! `X-SpedHub-Version`, der Server antwortet beim Anmelden mit `server_version` und `compatible`.

/// Version dieses Servers, aus der `Cargo.toml` im Wurzelordner.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Header, in dem Launcher und Plugin ihre Version schicken.
pub const HEADER: &str = "x-spedhub-version";

/// Haupt- und Nebenversion aus `0.2.0`, `v0.2.0` oder `0.2.0-rc1`. `None` bei Unsinn.
pub fn major_minor(version: &str) -> Option<(u64, u64)> {
    let v = version.trim();
    let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

/// Passen zwei Versionen zusammen? Eine Version, die sich nicht lesen lässt, passt nie.
pub fn compatible(a: &str, b: &str) -> bool {
    match (major_minor(a), major_minor(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haupt_und_nebenversion() {
        assert_eq!(major_minor("0.2.0"), Some((0, 2)));
        assert_eq!(major_minor(" v1.10.3 "), Some((1, 10)));
        assert_eq!(major_minor("0.3.0-rc1"), Some((0, 3)));
        for bad in ["", "0.2", "0.2.x", "a.b.c", "0.2.0.1", "v", "-1.2.3"] {
            assert_eq!(major_minor(bad), None, "{bad}");
        }
        assert!(compatible("0.2.0", "0.2.9"));
        assert!(compatible("v0.2.0", "0.2.0"));
        assert!(!compatible("0.2.0", "0.3.0"));
        assert!(!compatible("1.2.0", "0.2.0"));
        assert!(!compatible("Unsinn", "0.2.0"));
    }
}
