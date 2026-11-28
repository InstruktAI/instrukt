//! Whitespace tokenization, random 3-to-8-word fragmentation, and CSPRNG
//! Fisher-Yates shuffling for untrusted-text screening
//! (`project/instrukt/spec/domain/screening`).

use ring::rand::{SecureRandom as _, SystemRandom};

use crate::_client::CallDefect;

const MIN_FRAGMENT_WORDS: usize = 3;
const MAX_FRAGMENT_WORDS: usize = 8;
const MIN_MULTI_FRAGMENT_WORDS: usize = MIN_FRAGMENT_WORDS * 2;
const MAX_SHUFFLE_ATTEMPTS: usize = 64;

/// Tokenizes `text` by whitespace and prepares fragments for screening:
/// - `0` words -> `Ok(None)` (empty/whitespace-only refusal at entry)
/// - `1..=5` words -> `Ok(Some([text]))` (single fragment)
/// - `>= 6` words -> `Ok(Some(shuffled))` of `>= 2` consecutive 3-to-8-word
///   fragments in CSPRNG random order guaranteed to differ from original order.
pub(crate) fn prepare_fragments(text: &str) -> Result<Option<Vec<String>>, CallDefect> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Ok(None);
    }
    if words.len() < MIN_MULTI_FRAGMENT_WORDS {
        return Ok(Some(vec![words.join(" ")]));
    }

    let rng = SystemRandom::new();
    let chunks = partition_words(&words, &rng)?;
    let shuffled = shuffle_non_identity(chunks, &rng)?;
    Ok(Some(shuffled))
}

fn partition_words(words: &[&str], rng: &SystemRandom) -> Result<Vec<String>, CallDefect> {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    while offset < words.len() {
        let remaining = words.len() - offset;
        let take = if !chunks.is_empty() && remaining <= MAX_FRAGMENT_WORDS {
            remaining
        } else {
            let max_take = MAX_FRAGMENT_WORDS.min(remaining - MIN_FRAGMENT_WORDS);
            let span = max_take - MIN_FRAGMENT_WORDS + 1;
            MIN_FRAGMENT_WORDS + random_below(rng, span)?
        };
        chunks.push(words[offset..offset + take].join(" "));
        offset += take;
    }
    Ok(chunks)
}

fn shuffle_non_identity(
    chunks: Vec<String>,
    rng: &SystemRandom,
) -> Result<Vec<String>, CallDefect> {
    let len = chunks.len();
    debug_assert!(len >= 2);
    let all_identical = chunks.windows(2).all(|pair| pair[0] == pair[1]);
    let mut indices: Vec<usize> = (0..len).collect();

    for _ in 0..MAX_SHUFFLE_ATTEMPTS {
        for i in (1..len).rev() {
            let j = random_below(rng, i + 1)?;
            indices.swap(i, j);
        }
        let non_identity = indices.iter().enumerate().any(|(pos, &orig)| pos != orig);
        let order_changed = all_identical
            || indices
                .iter()
                .enumerate()
                .any(|(pos, &orig)| chunks[orig] != chunks[pos]);
        if non_identity && order_changed {
            return Ok(indices.into_iter().map(|i| chunks[i].clone()).collect());
        }
    }

    // Fallback (unreachable with probability > 1 - 2^-64) guarantees non-identity
    // order without unbounded looping.
    let mut rotated: Vec<usize> = (0..len).collect();
    let shift = if all_identical {
        1
    } else {
        (1..len)
            .find(|&s| (0..len).any(|i| chunks[(i + s) % len] != chunks[i]))
            .unwrap_or(1)
    };
    rotated.rotate_left(shift);
    Ok(rotated.into_iter().map(|i| chunks[i].clone()).collect())
}

/// Unbiased CSPRNG integer in `0..bound` via rejection sampling.
fn random_below(rng: &SystemRandom, bound: usize) -> Result<usize, CallDefect> {
    if bound <= 1 {
        return Ok(0);
    }
    let bound_u64 = bound as u64;
    let limit = (u64::MAX / bound_u64) * bound_u64;
    let mut buf = [0u8; 8];
    loop {
        rng.fill(&mut buf).map_err(|_| CallDefect::RandomSource)?;
        let sample = u64::from_le_bytes(buf);
        if sample < limit {
            return Ok((sample % bound_u64) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn empty_and_whitespace_only_input_returns_none() {
        assert_eq!(prepare_fragments("").unwrap(), None);
        assert_eq!(prepare_fragments("   \t\r\n  ").unwrap(), None);
    }

    #[test]
    fn one_to_five_words_produces_single_fragment() {
        for count in 1..=5 {
            let input = (1..=count)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join("  ");
            let expected = (1..=count)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(prepare_fragments(&input).unwrap(), Some(vec![expected]));
        }
    }

    #[test]
    fn six_to_hundred_words_always_satisfies_chunk_bounds_multiset_and_non_identity_shuffle() {
        for count in 6..=100 {
            let words: Vec<String> = (0..count).map(|i| format!("tok{i}")).collect();
            let input = words.join(" ");
            let fragments = prepare_fragments(&input).unwrap().unwrap();
            assert!(
                fragments.len() >= 2,
                "count={count} produced <2 fragments: {fragments:?}"
            );
            let mut reconstructed_multiset: BTreeMap<String, usize> = BTreeMap::new();
            for fragment in &fragments {
                let frag_words: Vec<&str> = fragment.split_whitespace().collect();
                assert!(
                    (MIN_FRAGMENT_WORDS..=MAX_FRAGMENT_WORDS).contains(&frag_words.len()),
                    "count={count}: fragment len {} outside 3..=8: {fragment:?}",
                    frag_words.len()
                );
                for w in frag_words {
                    *reconstructed_multiset.entry(w.to_owned()).or_default() += 1;
                }
            }
            let expected_multiset: BTreeMap<String, usize> =
                words.iter().map(|w| (w.clone(), 1)).collect();
            assert_eq!(reconstructed_multiset, expected_multiset);
            assert_ne!(
                fragments.join(" "),
                input,
                "count={count}: shuffled fragments matched original order"
            );
        }
    }

    #[test]
    fn identical_word_texts_terminate_cleanly_with_valid_chunk_bounds() {
        let input = std::iter::repeat_n("same", 24)
            .collect::<Vec<_>>()
            .join(" ");
        let fragments = prepare_fragments(&input).unwrap().unwrap();
        assert!(fragments.len() >= 2);
        let total: usize = fragments
            .iter()
            .map(|f| {
                let n = f.split_whitespace().count();
                assert!((MIN_FRAGMENT_WORDS..=MAX_FRAGMENT_WORDS).contains(&n));
                n
            })
            .sum();
        assert_eq!(total, 24);
    }
}
