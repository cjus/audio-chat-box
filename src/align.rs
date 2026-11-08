//! Word times for speech the node has made (`voice/speak` with `wordTimes`, spec 8.2): which
//! word of the text is said when.
//!
//! ## How it works
//!
//! This is forced alignment the way WhisperX does it: a speech model hears the audio, and the
//! text is lined up against what it heard. Kokoro, which makes the speech, doesn't say when
//! it says each word, but Parakeet, which hears, does: its transducer writes word pieces and
//! predicts how long each lasts ([`Piece`]). So [`align`] takes the text the client asked to
//! be read and the pieces Parakeet heard in the result, and says when each word of the text
//! was said.
//!
//! The two never match word for word. Parakeet writes "4" where the text said "four", "C
//! pawn" for "c-pawn", "bishop" for "bishop", and may mishear a word. So the words aren't
//! compared: both sides are reduced to their letters (digits spelled out, [`letters`]) and
//! lined up letter by letter with the least edits (Levenshtein distance). A letter of the text
//! lined up with a letter of what was heard takes that letter's time, which is its piece's
//! time spread evenly over the piece's letters. A letter with nothing lined up to it (a
//! mishearing, a word Parakeet dropped) takes a share of the gap between the letters either
//! side of it. A word starts when its first letter does and ends when its last does.
//!
//! ## Pitfalls
//!
//! - **The times are Parakeet's frames, 80 ms apart.** A word's start and end are accurate to
//!   about that. They are good for lighting a word as it is said, and for a move that is
//!   named in it, not for anything finer.
//! - **Not heard is not known.** If less than [`LEAST_AGREEMENT`] of the text's letters line up
//!   with a letter that was heard (the audio is silent, or Parakeet heard something else), no
//!   times are given, rather than a guess dressed up as a measurement. The client has its own
//!   estimate for that.
//! - **Words with no letters** (a dash on its own) have no sound: they start and end where the
//!   word before ends.
//! - **Times never go backwards.** A word starts no earlier than the word before ends, and
//!   ends no earlier than it starts, whatever the alignment found.
//!
//! WhisperX: <https://github.com/m-bain/whisperX> (BSD-2-Clause); only its idea is used.

use cnp::voice::WordTime;

/// One word piece Parakeet heard, and when: the transducer's own times.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The piece as the model writes it: a piece that starts a word starts with a space.
    pub text: String,
    /// Where it starts in the audio, in seconds.
    pub start: f32,
    /// How long it lasts, in seconds.
    pub duration: f32,
}

/// The share of the text's letters that must line up with a letter that was heard, below which
/// [`align`] gives no times (see the [module docs](self)).
pub const LEAST_AGREEMENT: f32 = 0.6;

/// The words of `text` (split at whitespace), as [`align`] times them.
pub fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// When each of `text`'s [`words`] is said in speech of `total` seconds that Parakeet heard as
/// `pieces`: one [`WordTime`] for each word, in order, with the word as the text wrote it. Empty
/// when the speech wasn't heard well enough to say (see the [module docs](self)).
pub fn align(text: &str, pieces: &[Piece], total: f32) -> Vec<WordTime> {
    let words = words(text);
    let spelled: Vec<Vec<char>> = words.iter().map(|word| letters(word)).collect();
    let said: Vec<char> = spelled.iter().flatten().copied().collect();
    let (heard, times) = heard_letters(pieces);
    if said.is_empty() || heard.is_empty() {
        return Vec::new();
    }
    let lined_up = line_up(&said, &heard);
    let agreed = lined_up
        .iter()
        .zip(&said)
        .filter(|(at, letter)| at.is_some_and(|j| heard[j] == **letter))
        .count();
    if (agreed as f32) < LEAST_AGREEMENT * said.len() as f32 {
        return Vec::new();
    }
    let spans = fill(
        lined_up
            .iter()
            .map(|at| at.map(|j| times[j]))
            .collect::<Vec<_>>(),
        total,
    );
    let mut cursor = 0;
    let mut previous_end = 0.0_f32;
    let mut result = Vec::with_capacity(words.len());
    for (word, letters) in words.iter().zip(&spelled) {
        let (mut start, mut end) = match letters.len() {
            0 => (previous_end, previous_end),
            n => (spans[cursor].0, spans[cursor + n - 1].1),
        };
        cursor += letters.len();
        start = start.max(previous_end).min(total.max(previous_end));
        end = end.max(start);
        previous_end = end;
        result.push(WordTime {
            text: (*word).to_string(),
            start_ms: millis(start),
            end_ms: millis(end),
        });
    }
    result
}

/// Seconds as whole milliseconds, not below 0.
fn millis(seconds: f32) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

/// `word` as the letters it is said with: lower case, nothing but letters, digits spelled out
/// ("g4" is "g" and "four").
pub fn letters(word: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut digits = c.to_string();
            while let Some(next) = chars.peek().filter(|d| d.is_ascii_digit()) {
                digits.push(*next);
                chars.next();
            }
            out.extend(spell_number(&digits).chars());
        } else if c.is_alphabetic() {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// A run of digits as the English that says it: a number up to 99 as one number, anything else
/// digit by digit.
fn spell_number(digits: &str) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    match digits.parse::<usize>() {
        Ok(n) if n < 20 && !digits.starts_with('0') || digits == "0" => ONES[n].to_string(),
        Ok(n) if (20..100).contains(&n) && !digits.starts_with('0') => {
            let tens = TENS[n / 10 - 2];
            match n % 10 {
                0 => tens.to_string(),
                ones => format!("{tens}{}", ONES[ones]),
            }
        }
        _ => digits
            .chars()
            .filter_map(|d| d.to_digit(10))
            .map(|d| ONES[d as usize])
            .collect(),
    }
}

/// The letters Parakeet heard, and each one's span in seconds (its piece's span, shared out
/// evenly among the piece's letters).
fn heard_letters(pieces: &[Piece]) -> (Vec<char>, Vec<(f32, f32)>) {
    let (mut heard, mut times) = (Vec::new(), Vec::new());
    for piece in pieces {
        let spelled = letters(&piece.text);
        let (start, duration) = (piece.start.max(0.0), piece.duration.max(0.0));
        let n = spelled.len() as f32;
        for (k, letter) in spelled.into_iter().enumerate() {
            heard.push(letter);
            times.push((
                start + duration * k as f32 / n,
                start + duration * (k as f32 + 1.0) / n,
            ));
        }
    }
    (heard, times)
}

/// Lines `said` up against `heard` with the fewest edits: for each letter of `said`, the index
/// of the letter of `heard` it is lined up with (equal or substituted), or `None` where it was
/// not heard at all.
fn line_up(said: &[char], heard: &[char]) -> Vec<Option<usize>> {
    let (n, m) = (said.len(), heard.len());
    let width = m + 1;
    let mut cost = vec![0_u32; (n + 1) * width];
    for (j, first) in cost.iter_mut().enumerate().take(m + 1) {
        *first = j as u32;
    }
    for i in 1..=n {
        cost[i * width] = i as u32;
        for j in 1..=m {
            let swap = cost[(i - 1) * width + j - 1] + u32::from(said[i - 1] != heard[j - 1]);
            let skip_said = cost[(i - 1) * width + j] + 1;
            let skip_heard = cost[i * width + j - 1] + 1;
            cost[i * width + j] = swap.min(skip_said).min(skip_heard);
        }
    }
    let mut at = vec![None; n];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let here = cost[i * width + j];
        let swap = cost[(i - 1) * width + j - 1] + u32::from(said[i - 1] != heard[j - 1]);
        if here == swap {
            at[i - 1] = Some(j - 1);
            i -= 1;
            j -= 1;
        } else if here == cost[(i - 1) * width + j] + 1 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    at
}

/// Gives every letter a span: a letter that was heard keeps its own, and a run of letters that
/// weren't share out the gap between the letters either side (the start of the speech and its
/// end, `total`, at the edges).
fn fill(known: Vec<Option<(f32, f32)>>, total: f32) -> Vec<(f32, f32)> {
    let mut spans = vec![(0.0, 0.0); known.len()];
    let mut i = 0;
    let mut gap_start = 0.0;
    while i < known.len() {
        if let Some(span) = known[i] {
            spans[i] = span;
            gap_start = span.1;
            i += 1;
            continue;
        }
        let run = known[i..].iter().take_while(|s| s.is_none()).count();
        let gap_end = known
            .get(i + run)
            .and_then(|s| s.map(|span| span.0))
            .unwrap_or(total)
            .max(gap_start);
        for k in 0..run {
            let share = (gap_end - gap_start) / run as f32;
            spans[i + k] = (
                gap_start + share * k as f32,
                gap_start + share * (k as f32 + 1.0),
            );
        }
        i += run;
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(text: &str, start: f32, duration: f32) -> Piece {
        Piece {
            text: text.to_string(),
            start,
            duration,
        }
    }

    #[test]
    fn digits_are_spelled_out() {
        let said = |word: &str| letters(word).into_iter().collect::<String>();
        assert_eq!(said("g4"), "gfour");
        assert_eq!(said("e5,"), "efive");
        assert_eq!(said("12"), "twelve");
        assert_eq!(said("45"), "fortyfive");
        assert_eq!(said("100"), "onezerozero");
        assert_eq!(said("c-pawn"), "cpawn");
        assert_eq!(said("—"), "");
    }

    #[test]
    fn each_word_is_timed_by_the_pieces_it_was_heard_in() {
        let heard = [
            piece(" Knight", 0.1, 0.4),
            piece(" to", 0.5, 0.2),
            piece(" f", 0.7, 0.1),
            piece("3", 0.8, 0.3),
        ];
        let timed = align("Knight to f3.", &heard, 1.2);
        let got: Vec<_> = timed
            .iter()
            .map(|w| (w.text.as_str(), w.start_ms, w.end_ms))
            .collect();
        assert_eq!(
            got,
            [("Knight", 100, 500), ("to", 500, 700), ("f3.", 700, 1100)]
        );
    }

    #[test]
    fn what_was_said_in_words_and_heard_in_digits_still_lines_up() {
        // The text says "four", Parakeet writes "4".
        let heard = [
            piece(" bishop", 0.0, 0.5),
            piece(" g", 0.5, 0.2),
            piece(" 4", 0.7, 0.3),
        ];
        let timed = align("bishop g four", &heard, 1.1);
        assert_eq!(timed.len(), 3);
        assert_eq!((timed[2].start_ms, timed[2].end_ms), (700, 1000));
    }

    #[test]
    fn a_word_that_is_split_or_joined_differently_keeps_its_times() {
        // "c-pawn" is one word of the text; Parakeet heard two.
        let heard = [piece(" C", 0.0, 0.2), piece(" pawn", 0.2, 0.4)];
        let timed = align("c-pawn", &heard, 0.7);
        assert_eq!((timed[0].start_ms, timed[0].end_ms), (0, 600));
    }

    #[test]
    fn a_word_that_was_not_heard_shares_the_gap_between_its_neighbours() {
        let heard = [piece(" Castle", 0.0, 0.5), piece(" important", 1.5, 0.5)];
        let timed = align("Castle soon important", &heard, 2.0);
        assert_eq!(timed.len(), 3);
        assert_eq!(timed[0].end_ms, 500);
        assert_eq!(timed[1].start_ms, 500);
        assert_eq!(timed[1].end_ms, 1500);
        assert_eq!(timed[2].start_ms, 1500);
    }

    #[test]
    fn times_never_go_backwards() {
        let heard = [piece(" one", 0.0, 0.4), piece(" two", 0.4, 0.4)];
        let timed = align("one — two", &heard, 0.9);
        assert_eq!(timed.len(), 3);
        let mut last = 0;
        for word in &timed {
            assert!(word.start_ms >= last && word.end_ms >= word.start_ms);
            last = word.end_ms;
        }
        // The dash has no sound: it starts and ends where "one" does.
        assert_eq!((timed[1].start_ms, timed[1].end_ms), (400, 400));
    }

    #[test]
    fn speech_that_was_not_heard_has_no_times() {
        assert!(align("Castle soon", &[], 1.0).is_empty());
        let wrong = [
            piece(" completely", 0.0, 0.5),
            piece(" different", 0.5, 0.5),
        ];
        assert!(align("Castle soon", &wrong, 1.0).is_empty());
    }
}
