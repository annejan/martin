//! Read a `MARTIN_*` tuning knob from the environment, WARNING on a set-but-unparseable value instead
//! of silently falling back to the default — so a typo (`MARTIN_ZOOM=abc`) tells you it was ignored
//! rather than vanishing without a trace.

use std::str::FromStr;

/// `key` parsed as `T`, or `default`. An **unset** var quietly uses the default (the normal case); a
/// var that is set but does **not** parse logs a warning and uses the default.
pub fn or<T: FromStr>(key: &str, default: T) -> T {
    match std::env::var(key) {
        Err(_) => default,
        Ok(s) => s.parse().unwrap_or_else(|_| {
            eprintln!(
                "env: ignoring {key}={s:?} — not a valid {}; using the default",
                std::any::type_name::<T>()
            );
            default
        }),
    }
}

/// `Some(key parsed as T)` when set+valid, `None` when **unset** (the knob is absent), and `None`+a
/// warning when set-but-unparseable. For optional knobs where "unset" is meaningfully distinct from any
/// value (e.g. `MARTIN_HOLD_T` — pin the timeline only when present).
pub fn opt<T: FromStr>(key: &str) -> Option<T> {
    let s = std::env::var(key).ok()?;
    match s.parse() {
        Ok(v) => Some(v),
        Err(_) => {
            eprintln!(
                "env: ignoring {key}={s:?} — not a valid {}; treating as unset",
                std::any::type_name::<T>()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unset_defaults_valid_parses_and_bad_falls_back() {
        // SAFETY: a unique key only this single-threaded test touches.
        let key = "MARTIN_TEST_ENVVAR_OR";
        unsafe { std::env::remove_var(key) };
        assert_eq!(super::or(key, 7u32), 7, "unset → default");
        unsafe { std::env::set_var(key, "42") };
        assert_eq!(super::or(key, 7u32), 42, "set + valid → parsed");
        unsafe { std::env::set_var(key, "nope") };
        assert_eq!(
            super::or(key, 7u32),
            7,
            "set + invalid → default (warns to stderr)"
        );
        unsafe { std::env::remove_var(key) };
    }
}
