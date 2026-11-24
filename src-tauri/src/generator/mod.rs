//! Password generator (OsRng): random characters or an EFF passphrase.
//!
//! Wordlist: EFF large wordlist (7776 words), © Electronic Frontier Foundation,
//! CC BY 3.0 US, <https://www.eff.org/dice>.

use rand::{rngs::OsRng, seq::SliceRandom, CryptoRng, Rng};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{AppError, AppResult};

const WORDLIST: &str = include_str!("eff_large_wordlist.txt");

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?/~";
/// Easily confused characters, dropped when `exclude_ambiguous` is set.
const AMBIGUOUS: &str = "Il1O0o|`'\"";

pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 128;
pub const MIN_WORDS: usize = 3;
pub const MAX_WORDS: usize = 20;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Options {
    Chars {
        length: usize,
        lower: bool,
        upper: bool,
        digits: bool,
        symbols: bool,
        exclude_ambiguous: bool,
    },
    Passphrase {
        words: usize,
        separator: String,
        capitalize: bool,
        add_digit: bool,
    },
}

#[derive(Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    pub password: String,
    /// Entropy of the generation process (not of the string), in bits.
    #[zeroize(skip)]
    pub entropy_bits: f64,
}

impl std::fmt::Debug for Generated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Generated")
            .field("entropy_bits", &self.entropy_bits)
            .finish_non_exhaustive()
    }
}

pub fn generate(options: &Options) -> AppResult<Generated> {
    generate_with(&mut OsRng, options)
}

pub fn generate_with(rng: &mut (impl Rng + CryptoRng), options: &Options) -> AppResult<Generated> {
    match options {
        Options::Chars {
            length,
            lower,
            upper,
            digits,
            symbols,
            exclude_ambiguous,
        } => chars(rng, *length, [*lower, *upper, *digits, *symbols], *exclude_ambiguous),
        Options::Passphrase {
            words,
            separator,
            capitalize,
            add_digit,
        } => passphrase(rng, *words, separator, *capitalize, *add_digit),
    }
}

fn chars(
    rng: &mut (impl Rng + CryptoRng),
    length: usize,
    enabled: [bool; 4],
    exclude_ambiguous: bool,
) -> AppResult<Generated> {
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        return Err(AppError::InvalidInput { field: "length" });
    }
    let sets: Vec<Vec<char>> = [LOWER, UPPER, DIGITS, SYMBOLS]
        .iter()
        .zip(enabled)
        .filter(|(_, on)| *on)
        .map(|(set, _)| {
            set.chars()
                .filter(|c| !exclude_ambiguous || !AMBIGUOUS.contains(*c))
                .collect()
        })
        .collect();
    if sets.is_empty() {
        return Err(AppError::InvalidInput { field: "sets" });
    }
    let pool: Vec<char> = sets.iter().flatten().copied().collect();

    // One character from each selected set, the rest from the whole pool, shuffled.
    let mut out: Vec<char> = sets
        .iter()
        .map(|set| *set.choose(rng).expect("sets are non-empty"))
        .collect();
    while out.len() < length {
        out.push(*pool.choose(rng).expect("pool is non-empty"));
    }
    out.shuffle(rng);
    // Sized up front: a growing `String` would leave freed, unwiped partial copies.
    let mut password = String::with_capacity(out.iter().map(|c| c.len_utf8()).sum());
    password.extend(out.iter());
    out.zeroize();
    Ok(Generated {
        password,
        entropy_bits: length as f64 * (pool.len() as f64).log2(),
    })
}

fn passphrase(
    rng: &mut (impl Rng + CryptoRng),
    words: usize,
    separator: &str,
    capitalize: bool,
    add_digit: bool,
) -> AppResult<Generated> {
    if !(MIN_WORDS..=MAX_WORDS).contains(&words) {
        return Err(AppError::InvalidInput { field: "words" });
    }
    if separator.chars().count() > 3 || separator.chars().any(char::is_control) {
        return Err(AppError::InvalidInput { field: "separator" });
    }
    let list: Vec<&str> = WORDLIST.lines().collect();
    let mut picked: Vec<String> = (0..words)
        .map(|_| {
            let w = *list.choose(rng).expect("wordlist is non-empty");
            // Room for a capitalized first letter (up to 3 chars) and the optional
            // digit, so neither step reallocates and leaves a stray copy.
            let mut word = String::with_capacity(w.len() + 4 * 3 + 1);
            let mut c = w.chars();
            if capitalize {
                word.extend(c.next().into_iter().flat_map(char::to_uppercase));
            }
            word.extend(c);
            word
        })
        .collect();
    let mut entropy = words as f64 * (list.len() as f64).log2();
    if add_digit {
        let i = rng.gen_range(0..words);
        picked[i].push(char::from(b'0' + rng.gen_range(0..10u8)));
        entropy += 10f64.log2();
    }
    let password = picked.join(separator);
    picked.zeroize();
    Ok(Generated {
        password,
        entropy_bits: entropy,
    })
}

#[cfg(test)]
mod tests {
    use rand::{rngs::StdRng, SeedableRng};

    use super::*;

    fn chars_opts(length: usize, sets: [bool; 4], exclude_ambiguous: bool) -> Options {
        Options::Chars {
            length,
            lower: sets[0],
            upper: sets[1],
            digits: sets[2],
            symbols: sets[3],
            exclude_ambiguous,
        }
    }

    #[test]
    fn wordlist_is_the_eff_large_list() {
        let list: Vec<&str> = WORDLIST.lines().collect();
        assert_eq!(list.len(), 7776);
        assert_eq!((list[0], list[7775]), ("abacus", "zoom"));
        let unique: std::collections::HashSet<_> = list.iter().collect();
        assert_eq!(unique.len(), 7776);
    }

    #[test]
    fn chars_respect_length_and_include_every_set() {
        for _ in 0..200 {
            let g = generate(&chars_opts(12, [true, true, true, true], false)).unwrap();
            let pw = &g.password;
            assert_eq!(pw.chars().count(), 12);
            assert!(pw.chars().any(|c| c.is_ascii_lowercase()));
            assert!(pw.chars().any(|c| c.is_ascii_uppercase()));
            assert!(pw.chars().any(|c| c.is_ascii_digit()));
            assert!(pw.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn ambiguous_characters_can_be_excluded() {
        for _ in 0..200 {
            let g = generate(&chars_opts(64, [true, true, true, true], true)).unwrap();
            assert!(!g.password.chars().any(|c| AMBIGUOUS.contains(c)), "{}", g.password);
        }
    }

    #[test]
    fn digits_only_and_entropy() {
        let g = generate(&chars_opts(10, [false, false, true, false], false)).unwrap();
        assert!(g.password.chars().all(|c| c.is_ascii_digit()));
        assert!((g.entropy_bits - 10.0 * 10f64.log2()).abs() < 1e-9);
    }

    #[test]
    fn invalid_options_are_rejected() {
        let bad = |o: Options| generate(&o).unwrap_err();
        assert_eq!(
            bad(chars_opts(7, [true; 4], false)),
            AppError::InvalidInput { field: "length" }
        );
        assert_eq!(
            bad(chars_opts(129, [true; 4], false)),
            AppError::InvalidInput { field: "length" }
        );
        assert_eq!(
            bad(chars_opts(16, [false; 4], false)),
            AppError::InvalidInput { field: "sets" }
        );
        let pp = |words, sep: &str| Options::Passphrase {
            words,
            separator: sep.into(),
            capitalize: false,
            add_digit: false,
        };
        assert_eq!(bad(pp(2, "-")), AppError::InvalidInput { field: "words" });
        assert_eq!(bad(pp(5, "----")), AppError::InvalidInput { field: "separator" });
    }

    #[test]
    fn passphrase_shape_and_entropy() {
        let opts = Options::Passphrase {
            words: 5,
            separator: "-".into(),
            capitalize: true,
            add_digit: true,
        };
        let g = generate_with(&mut StdRng::seed_from_u64(1), &opts).unwrap();
        let parts: Vec<&str> = g.password.split('-').collect();
        assert_eq!(parts.len(), 5, "{}", g.password);
        assert!(parts.iter().all(|w| w.chars().next().unwrap().is_uppercase()));
        assert_eq!(g.password.chars().filter(char::is_ascii_digit).count(), 1);
        assert!((g.entropy_bits - (5.0 * 7776f64.log2() + 10f64.log2())).abs() < 1e-9);
        // ~64.6 + 3.3 bits
        assert!(g.entropy_bits > 67.0);
    }

    #[test]
    fn output_is_built_without_reallocation_and_not_in_debug() {
        let g = generate(&chars_opts(64, [true; 4], false)).unwrap();
        assert_eq!(g.password.len(), g.password.capacity());
        assert!(!format!("{g:?}").contains(&g.password));
        let opts = Options::Passphrase {
            words: 6,
            separator: "-".into(),
            capitalize: true,
            add_digit: true,
        };
        let g = generate(&opts).unwrap();
        assert_eq!(g.password.len(), g.password.capacity(), "join sizes its buffer exactly");
        assert!(!format!("{g:?}").contains(&g.password));
    }

    #[test]
    fn output_varies() {
        let opts = chars_opts(20, [true; 4], false);
        let a = generate(&opts).unwrap();
        let b = generate(&opts).unwrap();
        assert_ne!(a.password, b.password);
    }
}
