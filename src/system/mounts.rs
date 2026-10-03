//! Required-mount checks. Reads /proc/self/mountinfo with hard size limits;
//! unescapes kernel octal escapes (\040 etc.) before comparing.

use std::fs::File;
use std::io::Read;

/// Never read more than this from mountinfo (defense against a pathological
/// procfs).
const MAX_MOUNTINFO_BYTES: u64 = 1 << 20;

pub fn mount_points() -> Vec<String> {
    let mut s = String::new();
    if let Ok(mut f) = File::open("/proc/self/mountinfo") {
        let _ = f.take(MAX_MOUNTINFO_BYTES).read_to_string(&mut s);
    }
    s.lines()
        .filter_map(|line| {
            // mountinfo: "ID parent major:minor root MOUNTPOINT options - fstype ..."
            let before_dash = line.split(" - ").next()?;
            before_dash.split_whitespace().nth(4).map(unescape)
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut v = 0u32;
            let mut n = 0;
            while n < 3 {
                if let Some(&d) = chars.peek() {
                    if let Some(dig) = d.to_digit(8) {
                        v = v * 8 + dig;
                        chars.next();
                        n += 1;
                        continue;
                    }
                }
                break;
            }
            if n > 0 { out.push(char::from_u32(v).unwrap_or(' ')); }
            else { out.push('\\'); }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn octal_unescape() {
        assert_eq!(unescape("/run/media/My\\040Disk"), "/run/media/My Disk");
        assert_eq!(unescape("/plain/path"), "/plain/path");
        assert_eq!(unescape("trailing\\"), "trailing\\");
    }
}