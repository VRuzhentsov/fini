//! Rank keys: the manual order of the quests (docs/glossary.md, "Rank").
//!
//! A rank key is text. A key with lower bytes comes first (byte comparison,
//! as `str::cmp` and SQLite's `BINARY` collation do). Between two different
//! keys there is always one more key, so a move writes only the quest that
//! moves: no other key changes.
//!
//! A key is an integer part and a fraction part. The first character of the
//! integer part gives its length: `a` is followed by 1 digit, `b` by 2, up
//! to `z` by 26; `Z` is followed by 1 digit, `Y` by 2, down to `A` by 26.
//! The `A`..`Z` keys come before `a0`. The digits are base 62, `0-9A-Za-z`,
//! which is also their byte order. A key at the top or the bottom of the
//! list adds 1 to the integer part, so these keys stay short. A key between
//! two keys with the same integer part takes the middle of the two fraction
//! parts, which adds about one digit for each six moves to the same place.
//! The fraction part has no `0` as its last digit: then there is always a
//! key before it.
//!
//! When a key becomes longer than `Rank::MAX_KEY_LEN`, or when two
//! neighbours have the same key, the caller gives all quests new keys with
//! `Rank::spread` (a rebalance). This occurs rarely.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;
/// The key of an empty list.
const FIRST_KEY: &str = "a0";
/// The smallest integer part: `A` and 26 zeros. No key comes before it, so
/// it is not a key.
const SMALLEST_INTEGER: &str = "A00000000000000000000000000";
/// The integer part that `spread` and `from_legacy` give: `c` and 3 digits.
const SPREAD_HEAD: u8 = b'c';
const SPREAD_CAPACITY: usize = BASE * BASE * BASE;

/// The rank keys of the quests. See the module documentation.
pub struct Rank;

impl Rank {
    /// The longest key that a move keeps. A longer key starts a rebalance.
    pub const MAX_KEY_LEN: usize = 32;

    /// A key after `lower` and before `upper`. `None` is the start or the
    /// end of the list: `between(None, None)` is the key of an empty list.
    /// An error when a key is not valid, or when `lower` is not before
    /// `upper` (two neighbours with the same key).
    pub fn between(lower: Option<&str>, upper: Option<&str>) -> Result<String, String> {
        if let Some(lower) = lower {
            Self::validate(lower)?;
        }
        if let Some(upper) = upper {
            Self::validate(upper)?;
        }
        match (lower, upper) {
            (None, None) => Ok(FIRST_KEY.to_string()),
            (None, Some(upper)) => Self::before(upper),
            (Some(lower), None) => Ok(Self::after(lower)),
            (Some(lower), Some(upper)) if lower >= upper => Err(format!(
                "rank key {lower:?} is not before rank key {upper:?}"
            )),
            (Some(lower), Some(upper)) => Ok(Self::inside(lower, upper)),
        }
    }

    /// `count` keys in order, with equal space between them: the keys of a
    /// rebalance. The same count always gives the same keys.
    pub fn spread(count: usize) -> Vec<String> {
        if count + 1 < SPREAD_CAPACITY {
            let step = SPREAD_CAPACITY / (count + 1);
            return (1..=count).map(|index| spread_key(index * step)).collect();
        }
        let mut keys: Vec<String> = Vec::with_capacity(count);
        for _ in 0..count {
            let next = keys
                .last()
                .map_or_else(|| FIRST_KEY.to_string(), |last| Self::after(last));
            keys.push(next);
        }
        keys
    }

    /// The key of a quest from before rank keys, from its decimal
    /// `order_rank` in [-100, 100]. Each device gives the same key to the
    /// same value, so that devices that sync keep the same order. Values
    /// with less than 0.001 between them get the same key. Migration
    /// `quest_rank_key` uses the same formula in SQL.
    pub fn from_legacy(value: f64) -> String {
        let value = if value.is_nan() { 0.0 } else { value };
        // The result is in [0, 200000], below SPREAD_CAPACITY.
        let position = ((value.clamp(-100.0, 100.0) + 100.0) * 1000.0).round() as usize;
        spread_key(position)
    }

    /// An error when `key` is not a rank key.
    pub fn validate(key: &str) -> Result<(), String> {
        let bytes = key.as_bytes();
        if bytes.iter().any(|byte| digit_value(*byte).is_none()) {
            return Err(format!(
                "rank key {key:?} has a character that is not a digit"
            ));
        }
        let integer = integer_part(bytes)?;
        if key == SMALLEST_INTEGER {
            return Err(format!("rank key {key:?} is the smallest integer part"));
        }
        if bytes.len() > integer.len() && bytes.last() == Some(&b'0') {
            return Err(format!("rank key {key:?} ends with 0"));
        }
        Ok(())
    }

    /// A key before the valid key `upper`.
    fn before(upper: &str) -> Result<String, String> {
        let upper = upper.as_bytes();
        let integer = integer_part(upper)?;
        let fraction = &upper[integer.len()..];
        if integer == SMALLEST_INTEGER.as_bytes() {
            return Ok(text(
                [integer, midpoint(&[], Some(fraction)).as_slice()].concat(),
            ));
        }
        if integer.len() < upper.len() {
            // `upper` has a fraction part, so its integer part alone is before it.
            return Ok(text(integer.to_vec()));
        }
        decrement_integer(integer)
            .map(text)
            .ok_or_else(|| "no rank key is before the smallest integer part".to_string())
    }

    /// A key after the valid key `lower`.
    fn after(lower: &str) -> String {
        let lower = lower.as_bytes();
        // `lower` is valid, so its integer part is there.
        let integer = integer_part(lower).unwrap_or(lower);
        let fraction = &lower[integer.len()..];
        match increment_integer(integer) {
            Some(next) => text(next),
            None => text([integer, midpoint(fraction, None).as_slice()].concat()),
        }
    }

    /// A key between the valid keys `lower` < `upper`.
    fn inside(lower: &str, upper: &str) -> String {
        let (lower, upper) = (lower.as_bytes(), upper.as_bytes());
        let lower_integer = integer_part(lower).unwrap_or(lower);
        let upper_integer = integer_part(upper).unwrap_or(upper);
        let lower_fraction = &lower[lower_integer.len()..];
        if lower_integer == upper_integer {
            let upper_fraction = &upper[upper_integer.len()..];
            let middle = midpoint(lower_fraction, Some(upper_fraction));
            return text([lower_integer, middle.as_slice()].concat());
        }
        match increment_integer(lower_integer) {
            Some(next) if next.as_slice() < upper => text(next),
            _ => text([lower_integer, midpoint(lower_fraction, None).as_slice()].concat()),
        }
    }
}

/// The value of a base-62 digit.
fn digit_value(byte: u8) -> Option<usize> {
    match byte {
        b'0'..=b'9' => Some(usize::from(byte - b'0')),
        b'A'..=b'Z' => Some(usize::from(byte - b'A') + 10),
        b'a'..=b'z' => Some(usize::from(byte - b'a') + 36),
        _ => None,
    }
}

/// The integer part of `key`: its first character and the digits that the
/// first character gives.
fn integer_part(key: &[u8]) -> Result<&[u8], String> {
    let length = match key.first() {
        Some(head @ b'a'..=b'z') => usize::from(head - b'a') + 2,
        Some(head @ b'A'..=b'Z') => usize::from(b'Z' - head) + 2,
        _ => {
            return Err(format!(
                "rank key {:?} has no integer part",
                text(key.to_vec())
            ))
        }
    };
    key.get(..length)
        .ok_or_else(|| format!("rank key {:?} is too short", text(key.to_vec())))
}

/// The fraction digits in the middle of `lower` and `upper` (`None`: 1).
/// `lower` < `upper`, and neither ends with `0`.
fn midpoint(lower: &[u8], upper: Option<&[u8]>) -> Vec<u8> {
    if let Some(upper) = upper {
        // The digits that are the same at the start (a missing digit of `lower` is 0).
        let shared = upper
            .iter()
            .enumerate()
            .take_while(|(index, digit)| lower.get(*index).copied().unwrap_or(b'0') == **digit)
            .count();
        if shared > 0 {
            let rest = midpoint(lower.get(shared..).unwrap_or(&[]), Some(&upper[shared..]));
            return [&upper[..shared], rest.as_slice()].concat();
        }
    }
    let lower_digit = lower
        .first()
        .and_then(|byte| digit_value(*byte))
        .unwrap_or(0);
    let upper_digit = upper
        .and_then(|upper| upper.first())
        .and_then(|byte| digit_value(*byte))
        .unwrap_or(BASE);
    if upper_digit - lower_digit > 1 {
        return vec![DIGITS[(lower_digit + upper_digit).div_ceil(2)]];
    }
    // The two first digits are next to each other.
    if let Some(upper) = upper.filter(|upper| upper.len() > 1) {
        return vec![upper[0]];
    }
    let rest = midpoint(lower.get(1..).unwrap_or(&[]), None);
    [&[DIGITS[lower_digit]][..], rest.as_slice()].concat()
}

/// The next integer part, or `None` after the largest one.
fn increment_integer(integer: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = integer.split_first()?;
    let mut digits = digits.to_vec();
    for digit in digits.iter_mut().rev() {
        let value = digit_value(*digit)? + 1;
        if value < BASE {
            *digit = DIGITS[value];
            return Some([&[*head][..], digits.as_slice()].concat());
        }
        *digit = b'0';
    }
    // All digits were `z`: the next length.
    match *head {
        b'Z' => Some(FIRST_KEY.as_bytes().to_vec()),
        b'z' => None,
        head => {
            let next = head + 1;
            if next > b'a' {
                digits.push(b'0');
            } else {
                digits.pop();
            }
            Some([&[next][..], digits.as_slice()].concat())
        }
    }
}

/// The integer part before `integer`, or `None` before the smallest one.
fn decrement_integer(integer: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = integer.split_first()?;
    let mut digits = digits.to_vec();
    for digit in digits.iter_mut().rev() {
        let value = digit_value(*digit)?;
        if value > 0 {
            *digit = DIGITS[value - 1];
            return Some([&[*head][..], digits.as_slice()].concat());
        }
        *digit = b'z';
    }
    // All digits were `0`: the length before.
    match *head {
        b'a' => Some(b"Zz".to_vec()),
        b'A' => None,
        head => {
            let next = head - 1;
            if next < b'Z' {
                digits.push(b'z');
            } else {
                digits.pop();
            }
            Some([&[next][..], digits.as_slice()].concat())
        }
    }
}

/// The key `c` and 3 digits for `position` < `SPREAD_CAPACITY`.
fn spread_key(position: usize) -> String {
    text(vec![
        SPREAD_HEAD,
        DIGITS[position / (BASE * BASE) % BASE],
        DIGITS[position / BASE % BASE],
        DIGITS[position % BASE],
    ])
}

/// The digits as text. They are all ASCII.
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn between(lower: Option<&str>, upper: Option<&str>) -> String {
        Rank::between(lower, upper).expect("a key between the two keys")
    }

    #[test]
    fn the_key_of_an_empty_list_is_a0() {
        assert_eq!(between(None, None), "a0");
    }

    #[test]
    fn keys_at_the_end_add_one_to_the_integer_part() {
        assert_eq!(between(Some("a0"), None), "a1");
        assert_eq!(between(Some("az"), None), "b00");
        assert_eq!(between(Some("b0z"), None), "b10");
        assert_eq!(between(Some("Zz"), None), "a0");
        assert_eq!(between(Some("a0V"), None), "a1");
    }

    #[test]
    fn keys_at_the_start_subtract_one_from_the_integer_part() {
        assert_eq!(between(None, Some("a1")), "a0");
        assert_eq!(between(None, Some("a0")), "Zz");
        assert_eq!(between(None, Some("b00")), "az");
        assert_eq!(between(None, Some("Z0")), "Yzz");
        assert_eq!(between(None, Some("a0V")), "a0");
    }

    #[test]
    fn a_key_between_two_keys_takes_the_middle_of_the_fraction() {
        assert_eq!(between(Some("a0"), Some("a1")), "a0V");
        assert_eq!(between(Some("a0"), Some("a0V")), "a0G");
        assert_eq!(between(Some("a0V"), Some("a1")), "a0l");
        assert_eq!(between(Some("a0"), Some("a2")), "a1");
        assert_eq!(between(Some("a0z"), Some("a1")), "a0zV");
        assert_eq!(between(Some("a01"), Some("a02")), "a01V");
    }

    #[test]
    fn a_key_between_keys_of_different_lengths_is_in_order() {
        let key = between(Some("az"), Some("b00"));
        assert!("az" < key.as_str() && key.as_str() < "b00", "{key}");
        let key = between(Some("Zz"), Some("a0"));
        assert!("Zz" < key.as_str() && key.as_str() < "a0", "{key}");
    }

    #[test]
    fn rejects_neighbours_that_are_not_in_order() {
        assert!(Rank::between(Some("a1"), Some("a1")).is_err());
        assert!(Rank::between(Some("a2"), Some("a1")).is_err());
    }

    #[test]
    fn rejects_keys_that_are_not_valid() {
        for key in ["", "a", "b0", "a0 ", "a00", "a-1", "1.5", SMALLEST_INTEGER] {
            assert!(Rank::validate(key).is_err(), "{key:?} is not a key");
            assert!(Rank::between(Some(key), None).is_err(), "{key:?}");
        }
        for key in [
            "a0",
            "a0V",
            "Zz",
            "c0A1",
            "b00",
            "z00000000000000000000000000",
        ] {
            assert!(Rank::validate(key).is_ok(), "{key:?} is a key");
        }
    }

    /// Repeated moves into the same place (the worst case) and moves to the
    /// end and to the start keep the order, and keys stay in the bound for
    /// many moves.
    #[test]
    fn many_moves_keep_the_order() {
        let mut keys = vec![between(None, None)];
        for _ in 0..500 {
            let last = keys.last().cloned();
            keys.push(between(last.as_deref(), None));
            let first = keys.first().cloned();
            keys.insert(0, between(None, first.as_deref()));
        }
        let (mut lower, upper) = (keys[500].clone(), keys[501].clone());
        let mut inserted = 0;
        loop {
            let key = between(Some(&lower), Some(&upper));
            if key.len() > Rank::MAX_KEY_LEN {
                break;
            }
            assert!(lower < key && key < upper);
            lower = key;
            inserted += 1;
        }
        assert!(
            inserted > 100,
            "only {inserted} moves to one place before the bound"
        );
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(keys.iter().all(|key| key.len() <= 4), "{keys:?}");
    }

    /// A random sequence of moves between random neighbours, checked against
    /// the order of a plain list.
    #[test]
    fn random_moves_keep_the_order() {
        let mut seed: u64 = 0x5eed;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % (bound as u64)).unwrap_or(0)
        };
        let mut keys: Vec<String> = Vec::new();
        for _ in 0..2000 {
            let slot = next(keys.len() + 1);
            let lower = slot.checked_sub(1).map(|index| keys[index].as_str());
            let upper = keys.get(slot).map(String::as_str);
            let key = between(lower, upper);
            assert!(Rank::validate(&key).is_ok(), "{key:?}");
            keys.insert(slot, key);
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn spread_gives_keys_in_order_with_space_between() {
        assert!(Rank::spread(0).is_empty());
        let keys = Rank::spread(1000);
        assert_eq!(keys.len(), 1000);
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(keys
            .iter()
            .all(|key| key.len() == 4 && key.starts_with('c')));
        assert_eq!(Rank::spread(3), Rank::spread(3));
        // There is a short key between two spread keys.
        assert_eq!(between(Some(&keys[0]), Some(&keys[1])).len(), 4);
    }

    #[test]
    fn spread_of_more_keys_than_capacity_stays_in_order() {
        let keys = Rank::spread(SPREAD_CAPACITY);
        assert_eq!(keys.len(), SPREAD_CAPACITY);
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn legacy_values_keep_their_order() {
        let values = [
            -150.0, -100.0, -12.5, -0.001, 0.0, 0.0004, 0.001, 1.0, 12.5, 99.999, 100.0, 1000.0,
        ];
        let keys: Vec<String> = values
            .iter()
            .map(|value| Rank::from_legacy(*value))
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]), "{keys:?}");
        assert_eq!(Rank::from_legacy(-150.0), Rank::from_legacy(-100.0));
        assert_eq!(Rank::from_legacy(0.0), Rank::from_legacy(0.0004));
        assert!(Rank::from_legacy(0.0) < Rank::from_legacy(0.001));
        assert_eq!(Rank::from_legacy(f64::NAN), Rank::from_legacy(0.0));
        assert_eq!(Rank::from_legacy(-100.0), "c000");
        assert_eq!(Rank::from_legacy(0.0), "cQ0u");
        assert!(
            keys.iter().all(|key| Rank::validate(key).is_ok()),
            "{keys:?}"
        );
    }
}
