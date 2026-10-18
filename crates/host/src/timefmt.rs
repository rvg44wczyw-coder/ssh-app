//! Minimal ISO-8601 reader for the timestamps agents write (`2026-10-18T12:07:59.123Z`,
//! `...+02:00`). No time-zone database: a stamp without an offset counts as UTC.

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the epoch, or `None` when the text is not a valid stamp.
pub fn iso_epoch(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0i64, |acc, d| acc * 10 + i64::from(d - b'0'))
        })
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !(1..=12).contains(&mo) {
        return None;
    }
    if !(1..=31).contains(&d) || !matches!(b.get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if b.get(13) != Some(&b':') || b.get(16) != Some(&b':') || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac = 0.0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
        frac = format!("0.{}", &s[start..i]).parse().ok()?;
    }
    let offset = match b.get(i) {
        None | Some(b'Z' | b'z') if i + 1 >= b.len() => 0,
        Some(sign @ (b'+' | b'-')) => {
            let oh = num(i + 1, 2)?;
            let (om, end) = if b.get(i + 3) == Some(&b':') {
                (num(i + 4, 2)?, i + 6)
            } else {
                (0, i + 3)
            };
            if end != b.len() || oh > 23 || om > 59 {
                return None;
            }
            let v = oh * 3600 + om * 60;
            if *sign == b'+' {
                v
            } else {
                -v
            }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset) as f64 + frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shapes_agents_write() {
        assert_eq!(iso_epoch("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_epoch("2026-10-18T12:07:59Z"), Some(1_792_325_279.0));
        assert_eq!(
            iso_epoch("2026-10-18T14:07:59+02:00"),
            iso_epoch("2026-10-18T12:07:59Z")
        );
        assert_eq!(iso_epoch("2026-10-18T12:07:59.500Z"), Some(1_792_325_279.5));
        assert_eq!(
            iso_epoch("2026-10-18T12:07:59"),
            iso_epoch("2026-10-18T12:07:59Z")
        );
        assert_eq!(iso_epoch("2000-02-29T00:00:00Z"), Some(951_782_400.0));
    }

    #[test]
    fn refuses_garbage() {
        for bad in [
            "",
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-10-18",
            "2026-10-18T25:00:00Z",
            "2026-10-18T12:07:59Zjunk",
            "2026-10-18T12:07:59.Z",
        ] {
            assert_eq!(iso_epoch(bad), None, "{bad}");
        }
    }
}
