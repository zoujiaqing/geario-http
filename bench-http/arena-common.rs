//! Identical application work for the native H1 and Hyper-on-geario baselines.
use serde::Deserialize;

#[derive(Deserialize)]
struct Query {
    a: Option<i64>,
    b: Option<i64>,
}

pub fn sum(query: &str, body: &[u8]) -> Result<i64, ()> {
    let query: Query = serde_urlencoded::from_str(query).map_err(|_| ())?;
    let extra = std::str::from_utf8(body)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    query
        .a
        .unwrap_or(0)
        .checked_add(query.b.unwrap_or(0))
        .and_then(|n| n.checked_add(extra))
        .ok_or(())
}

#[cfg(test)]
mod tests {
    use super::sum;

    #[test]
    fn computes_inputs_not_a_fixed_response() {
        assert_eq!(sum("a=3&b=4", b""), Ok(7));
        assert_eq!(sum("a=10&b=-2", b" 5\n"), Ok(13));
        assert_eq!(sum("%61=11&b=2", b"9"), Ok(22));
        assert_eq!(sum("", b"-5"), Ok(-5));
    }

    #[test]
    fn rejects_invalid_query_and_overflow() {
        assert!(sum("a=no", b"").is_err());
        assert!(sum("a=9223372036854775807&b=1", b"").is_err());
    }
}
