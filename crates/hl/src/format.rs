use anyhow::Result;

/// Count visible width (roughly) for ASCII-ish tables.
fn w(s: &str) -> usize {
    s.chars().count()
}

pub fn pad_left(s: &str, width: usize) -> String {
    let n = w(s);
    if n >= width {
        return s.to_string();
    }
    format!("{}{}", " ".repeat(width - n), s)
}

pub fn pad_right(s: &str, width: usize) -> String {
    let n = w(s);
    if n >= width {
        return s.to_string();
    }
    format!("{}{}", s, " ".repeat(width - n))
}

pub fn commas_i64(mut n: i64) -> String {
    let neg = n < 0;
    if neg {
        n = -n;
    }
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    let mut out: String = out.chars().rev().collect();
    if neg {
        out.insert(0, '-');
    }
    out
}

pub fn fmt_fixed_with_commas(v: f64, decimals: usize) -> String {
    let sign = if v.is_sign_negative() { "-" } else { "" };
    let v = v.abs();

    // Round to desired decimals.
    let pow = 10_f64.powi(decimals as i32);
    let rounded = (v * pow).round() / pow;

    let int_part = rounded.trunc() as i64;
    let frac = (rounded.fract() * pow).round() as i64;

    if decimals == 0 {
        return format!("{sign}{}", commas_i64(int_part));
    }

    // frac needs leading zeros.
    let frac_str = format!("{:0width$}", frac, width = decimals);
    format!("{sign}{}.{}", commas_i64(int_part), frac_str)
}

pub fn parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok()
}

pub fn fmt_num_str(s: &str, decimals: usize) -> String {
    match parse_f64(s) {
        Some(v) => fmt_fixed_with_commas(v, decimals),
        None => s.to_string(),
    }
}

pub fn table(headers: &[&str], rows: &[Vec<String>], right_align: &[bool]) -> Result<String> {
    anyhow::ensure!(headers.len() == right_align.len(), "headers/right_align mismatch");
    for r in rows {
        anyhow::ensure!(r.len() == headers.len(), "row width mismatch");
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| w(h)).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(w(c));
        }
    }

    let mut out = String::new();

    // Header
    for (i, h) in headers.iter().enumerate() {
        let cell = if right_align[i] {
            pad_left(h, widths[i])
        } else {
            pad_right(h, widths[i])
        };
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&cell);
    }
    out.push('\n');

    // Separator
    for (i, width) in widths.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&"-".repeat(*width));
    }
    out.push('\n');

    // Rows
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            let cell = if right_align[i] {
                pad_left(c, widths[i])
            } else {
                pad_right(c, widths[i])
            };
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&cell);
        }
        out.push('\n');
    }

    Ok(out)
}
