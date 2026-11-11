//! How a client says a word is said (`pronunciations`, CNP spec 8): the client's IPA, turned
//! into the symbols Kokoro was trained on.
//!
//! A client that knows how a name sounds (a player's, an opening's) sends it with
//! `voice/speak` as a word and its sounds in the International Phonetic Alphabet. Kokoro's
//! phoneme symbols are not IPA: they are misaki's, where a diphthong is one capital letter
//! (`A` is /eɪ/, `I` /aɪ/, `O` /oʊ/, `W` /aʊ/, `Y` /ɔɪ/), /tʃ/ and /dʒ/ are `ʧ` and `ʤ`, and
//! the American r is `ɹ`. [`kokoro_symbols`] makes that translation, so the protocol stays
//! independent of any one model.
//!
//! **How the words reach the model.** sherpa-onnx reads each word's symbols from a lexicon,
//! one line a word (`word p h o n e m e s`), and Kokoro's speech takes them from there. The
//! entries of one request travel in the generation config's `extra` map as the key
//! `pronunciations`, in that same line format ([`Pronunciations::lexicon_lines`]). The prebuilt
//! sherpa-onnx ignores the key, so this node offers the capability only when built with the
//! `pronunciations` feature, for a sherpa-onnx that reads it (the patched build without
//! eSpeak NG, cjus/chessgui#539).
//!
//! **Pitfall: an entry the model can't say is dropped, not refused.** The spec says so: one
//! entry whose symbol Kokoro has no sound for must never silence a whole reply. Entries that
//! are malformed (empty, or holding a space or a line break) are refused, as the spec says.

use cnp::voice::Pronunciation;

/// The most entries one request may carry. A reply names a handful of players and openings;
/// this is far above that and keeps a request's size bounded.
pub const MAX_PRONUNCIATIONS: usize = 256;

/// The symbols Kokoro takes as they are: the IPA letters its `tokens.txt` holds, the stress
/// and length marks. The capital letters it also holds (`A`, `I`, `O`, `W`, `Y`, ...) are its
/// own diphthong symbols and are only reached through [`DIGRAPHS`], never from the client's IPA.
const PASS_THROUGH: &str =
    "ᵊabcdefhijklmnopqstuvwxyzɑɐɒæβɔɕçɖðʤəɚɛɜɟɡɥɨɪʝɯɰŋɳɲɴøɸθœɹɻʁɽʂʃʈʧʊʋʌɣɤχʎʒʔˈˌːʰʲᵻ";

/// Two IPA letters that Kokoro writes as one symbol.
const DIGRAPHS: [(&str, &str); 8] = [
    ("eɪ", "A"),
    ("aɪ", "I"),
    ("oʊ", "O"),
    ("əʊ", "O"),
    ("aʊ", "W"),
    ("ɔɪ", "Y"),
    ("tʃ", "ʧ"),
    ("dʒ", "ʤ"),
];

/// IPA letters that Kokoro writes differently, one for one or as two symbols.
const RESPELLED: [(char, &str); 4] = [
    // ASCII g is the IPA "ɡ" (U+0261); English r in IPA is the approximant "ɹ".
    ('g', "ɡ"),
    ('r', "ɹ"),
    ('ɝ', "ɜ ɹ"),
    ('ɫ', "l"),
];

/// Marks that join two letters or modify one, which Kokoro has no symbol for and a client may
/// well write: the tie bars of /t͡ʃ/ and /d͡ʒ/.
const IGNORED: [char; 2] = ['\u{0361}', '\u{035C}'];

/// Kokoro's symbols for the IPA in `ipa`, or `None` if some symbol has no sound in the model.
///
/// ```
/// use audio_chat_box::pronounce::kokoro_symbols;
/// assert_eq!(kokoro_symbols("ˈnɪmzoʊˌvɪtʃ").unwrap().join(" "), "ˈ n ɪ m z O ˌ v ɪ ʧ");
/// assert_eq!(kokoro_symbols("ˈkɑrpɔv").unwrap().join(" "), "ˈ k ɑ ɹ p ɔ v");
/// assert!(kokoro_symbols("ˈnɪm🙂").is_none());
/// ```
pub fn kokoro_symbols(ipa: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = ipa.chars().filter(|c| !IGNORED.contains(c)).collect();
    let mut symbols = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(next) = chars.get(i + 1) {
            let pair: String = [chars[i], *next].iter().collect();
            if let Some((_, one)) = DIGRAPHS.iter().find(|(two, _)| *two == pair) {
                symbols.push((*one).to_string());
                i += 2;
                continue;
            }
        }
        let c = chars[i];
        if let Some((_, to)) = RESPELLED.iter().find(|(from, _)| *from == c) {
            symbols.extend(to.split(' ').map(str::to_string));
        } else if PASS_THROUGH.contains(c) {
            symbols.push(c.to_string());
        } else {
            return None;
        }
        i += 1;
    }
    (!symbols.is_empty()).then_some(symbols)
}

/// The pronunciations of one `voice/speak` request, ready for the model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pronunciations(Vec<(String, Vec<String>)>);

impl Pronunciations {
    /// No pronunciations.
    pub fn none() -> Pronunciations {
        Pronunciations::default()
    }

    /// The pronunciations in `entries`, or why they can't be taken (for an `InvalidParams`
    /// error): an entry whose word or sounds are empty or hold a space or a line break, or more
    /// than [`MAX_PRONUNCIATIONS`]. An entry whose sounds Kokoro can't make is left out (module
    /// docs). Words are lowercased, since the model's lexicon is.
    pub fn of(entries: &[Pronunciation]) -> Result<Pronunciations, String> {
        if entries.len() > MAX_PRONUNCIATIONS {
            return Err(format!("at most {MAX_PRONUNCIATIONS} pronunciations"));
        }
        let mut taken = Vec::with_capacity(entries.len());
        for entry in entries {
            for (name, text) in [("word", &entry.word), ("ipa", &entry.ipa)] {
                if text.is_empty() {
                    return Err(format!("a pronunciation's {name} is empty"));
                }
                if text.contains(char::is_whitespace) || text.contains('\0') {
                    return Err(format!(
                        "the pronunciation {name} {text:?} holds a space or a line break"
                    ));
                }
            }
            if let Some(symbols) = kokoro_symbols(&entry.ipa) {
                taken.push((entry.word.to_lowercase(), symbols));
            }
        }
        Ok(Pronunciations(taken))
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many entries the model will say.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The entries as sherpa-onnx's lexicon reads them: a line a word, the word then its
    /// symbols, all separated by spaces.
    pub fn lexicon_lines(&self) -> String {
        self.0
            .iter()
            .map(|(word, symbols)| format!("{word} {}", symbols.join(" ")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(word: &str, ipa: &str) -> Pronunciation {
        Pronunciation {
            word: word.into(),
            ipa: ipa.into(),
        }
    }

    #[test]
    fn diphthongs_and_affricates_become_kokoros_single_symbols() {
        let said = |ipa| kokoro_symbols(ipa).unwrap().join(" ");
        assert_eq!(said("eɪaɪ"), "A I");
        assert_eq!(said("oʊ"), "O");
        assert_eq!(said("əʊ"), "O");
        assert_eq!(said("aʊ"), "W");
        assert_eq!(said("ɔɪ"), "Y");
        assert_eq!(said("tʃ"), "ʧ");
        assert_eq!(said("t͡ʃ"), "ʧ", "a tie bar is dropped");
        assert_eq!(said("dʒ"), "ʤ");
    }

    #[test]
    fn ascii_g_and_r_are_the_ipa_letters_kokoro_has() {
        assert_eq!(kokoro_symbols("græ").unwrap().join(" "), "ɡ ɹ æ");
        assert_eq!(kokoro_symbols("fɝ").unwrap().join(" "), "f ɜ ɹ");
    }

    #[test]
    fn kokoros_own_capital_symbols_are_not_ipa() {
        // "A" is Kokoro's /eɪ/, not a letter a client's IPA holds.
        assert_eq!(kokoro_symbols("A"), None);
        assert_eq!(kokoro_symbols("ˈnaɪt").unwrap().join(" "), "ˈ n I t");
    }

    #[test]
    fn a_sound_the_model_lacks_makes_no_symbols() {
        assert_eq!(kokoro_symbols("na🙂t"), None);
        assert_eq!(kokoro_symbols(""), None);
        assert_eq!(kokoro_symbols("\u{0361}"), None);
    }

    #[test]
    fn entries_become_lexicon_lines_with_lowercase_words() {
        let taken = Pronunciations::of(&[
            entry("Nimzowitsch", "ˈnɪmzoʊˌvɪtʃ"),
            entry("Kasparov", "ˈkæspərˌɔv"),
        ])
        .unwrap();
        assert_eq!(taken.len(), 2);
        assert_eq!(
            taken.lexicon_lines(),
            "nimzowitsch ˈ n ɪ m z O ˌ v ɪ ʧ\nkasparov ˈ k æ s p ə ɹ ˌ ɔ v"
        );
    }

    #[test]
    fn an_entry_the_model_cant_say_is_dropped_and_the_rest_kept() {
        let taken =
            Pronunciations::of(&[entry("emoji", "ˈɪmo🙂"), entry("karpov", "ˈkɑrpɔv")]).unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken.lexicon_lines(), "karpov ˈ k ɑ ɹ p ɔ v");
    }

    #[test]
    fn malformed_entries_are_refused() {
        for bad in [
            entry("", "naɪt"),
            entry("knight", ""),
            entry("two words", "naɪt"),
            entry("knight", "na\nɪt"),
            entry("knight", "naɪ t"),
        ] {
            assert!(
                Pronunciations::of(std::slice::from_ref(&bad)).is_err(),
                "{bad:?}"
            );
        }
        let many = vec![entry("a", "a"); MAX_PRONUNCIATIONS + 1];
        assert!(Pronunciations::of(&many).is_err());
        assert!(Pronunciations::of(&[]).unwrap().is_empty());
    }
}
