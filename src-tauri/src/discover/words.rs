//! Distinctive words of a group of notes: class-based TF-IDF (c-TF-IDF, as in
//! BERTopic). A word scores high in a group when it is frequent there and rare in the
//! other groups. Counted in memory only.

use std::collections::HashMap;

/// Words shorter than this (in characters) are skipped: mostly noise.
const MIN_LEN: usize = 3;
/// Longer "words" are hashes, base64 or URLs glued together.
const MAX_LEN: usize = 30;

/// Word counts of one group.
#[derive(Debug, Default, Clone)]
pub struct GroupWords {
    /// Per word: the sum over the group's notes of the word's share of that note's words.
    weights: HashMap<String, f64>,
    /// Notes counted that had any words.
    notes: usize,
}

impl GroupWords {
    /// Counts one note's words. Each note weighs the same however long it is, so one
    /// long note can't fill a group's words on its own.
    pub fn add_note(&mut self, text: &str) {
        let mut counts: HashMap<String, u32> = HashMap::new();
        let mut total = 0u32;
        for word in words(text) {
            *counts.entry(word).or_default() += 1;
            total += 1;
        }
        if total == 0 {
            return;
        }
        for (word, n) in counts {
            *self.weights.entry(word).or_default() += f64::from(n) / f64::from(total);
        }
        self.notes += 1;
    }
}

/// The lowercase words of `text` worth counting: letters and digits, at least
/// [`MIN_LEN`] long, not only digits, not an English stop word.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| (MIN_LEN..=MAX_LEN).contains(&w.chars().count()))
        .filter(|w| w.chars().any(char::is_alphabetic))
        .map(str::to_lowercase)
        .filter(|w| !is_stop_word(w))
}

/// The `top` most distinctive words of each group, best first.
///
/// Weight of word `t` in group `c`: `tf(t, c) · ln(1 + A / f(t))` (BERTopic's
/// c-TF-IDF), over notes normalised to the same length: `tf` is the word's average
/// share of the words of the group's notes, `A` the average number of notes per group
/// and `f(t)` the word's summed share over all notes. Ties go alphabetically.
pub fn distinctive(groups: &[GroupWords], top: usize) -> Vec<Vec<String>> {
    let mut overall: HashMap<&str, f64> = HashMap::new();
    for g in groups {
        for (word, w) in &g.weights {
            *overall.entry(word.as_str()).or_default() += w;
        }
    }
    let notes: usize = groups.iter().map(|g| g.notes).sum();
    let average = notes as f64 / groups.len().max(1) as f64;
    groups
        .iter()
        .map(|g| {
            let mut scored: Vec<(f64, &str)> = g
                .weights
                .iter()
                .map(|(word, w)| {
                    let tf = w / g.notes as f64;
                    let idf = (1.0 + average / overall[word.as_str()]).ln();
                    (tf * idf, word.as_str())
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
            scored
                .into_iter()
                .take(top)
                .map(|(_, w)| w.to_string())
                .collect()
        })
        .collect()
}

fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.binary_search(&word).is_ok()
}

/// English stop words (the common scikit-learn list, plus contractions split at the
/// apostrophe and bits of web addresses and markup), sorted for binary search.
const STOP_WORDS: &[&str] = &[
    "about",
    "above",
    "across",
    "after",
    "afterwards",
    "again",
    "against",
    "all",
    "almost",
    "alone",
    "along",
    "already",
    "also",
    "although",
    "always",
    "among",
    "amongst",
    "amount",
    "and",
    "another",
    "any",
    "anyhow",
    "anyone",
    "anything",
    "anyway",
    "anywhere",
    "are",
    "aren",
    "around",
    "back",
    "became",
    "because",
    "become",
    "becomes",
    "becoming",
    "been",
    "before",
    "beforehand",
    "behind",
    "being",
    "below",
    "beside",
    "besides",
    "between",
    "beyond",
    "both",
    "bottom",
    "but",
    "call",
    "can",
    "cannot",
    "cant",
    "com",
    "could",
    "couldn",
    "couldnt",
    "did",
    "didn",
    "does",
    "doesn",
    "doing",
    "don",
    "done",
    "down",
    "due",
    "during",
    "each",
    "eight",
    "either",
    "eleven",
    "else",
    "elsewhere",
    "empty",
    "enough",
    "etc",
    "even",
    "ever",
    "every",
    "everyone",
    "everything",
    "everywhere",
    "except",
    "few",
    "fifteen",
    "fifty",
    "fill",
    "find",
    "first",
    "five",
    "for",
    "former",
    "formerly",
    "forty",
    "four",
    "from",
    "front",
    "full",
    "further",
    "get",
    "give",
    "got",
    "had",
    "hadn",
    "has",
    "hasn",
    "hasnt",
    "have",
    "haven",
    "having",
    "hence",
    "her",
    "here",
    "hereafter",
    "hereby",
    "herein",
    "hereupon",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "however",
    "href",
    "html",
    "http",
    "https",
    "hundred",
    "inc",
    "indeed",
    "interest",
    "into",
    "isn",
    "its",
    "itself",
    "jpg",
    "just",
    "keep",
    "last",
    "latter",
    "latterly",
    "least",
    "less",
    "let",
    "ltd",
    "made",
    "many",
    "may",
    "meanwhile",
    "might",
    "mill",
    "mine",
    "more",
    "moreover",
    "most",
    "mostly",
    "move",
    "much",
    "must",
    "myself",
    "name",
    "namely",
    "neither",
    "never",
    "nevertheless",
    "next",
    "nine",
    "nobody",
    "none",
    "noone",
    "nor",
    "not",
    "nothing",
    "now",
    "nowhere",
    "off",
    "often",
    "once",
    "one",
    "only",
    "onto",
    "org",
    "other",
    "others",
    "otherwise",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "part",
    "per",
    "perhaps",
    "please",
    "png",
    "put",
    "rather",
    "really",
    "same",
    "see",
    "seem",
    "seemed",
    "seeming",
    "seems",
    "serious",
    "several",
    "she",
    "should",
    "shouldn",
    "show",
    "side",
    "since",
    "six",
    "sixty",
    "some",
    "somehow",
    "someone",
    "something",
    "sometime",
    "sometimes",
    "somewhere",
    "still",
    "such",
    "take",
    "ten",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "thence",
    "there",
    "thereafter",
    "thereby",
    "therefore",
    "therein",
    "thereupon",
    "these",
    "they",
    "thing",
    "things",
    "third",
    "this",
    "those",
    "though",
    "three",
    "through",
    "throughout",
    "thru",
    "thus",
    "together",
    "too",
    "top",
    "toward",
    "towards",
    "twelve",
    "twenty",
    "two",
    "under",
    "until",
    "upon",
    "use",
    "used",
    "very",
    "via",
    "was",
    "wasn",
    "way",
    "well",
    "were",
    "weren",
    "what",
    "whatever",
    "when",
    "whence",
    "whenever",
    "where",
    "whereafter",
    "whereas",
    "whereby",
    "wherein",
    "whereupon",
    "wherever",
    "whether",
    "which",
    "while",
    "whither",
    "who",
    "whoever",
    "whole",
    "whom",
    "whose",
    "why",
    "will",
    "with",
    "within",
    "without",
    "won",
    "would",
    "wouldn",
    "www",
    "yet",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_words_are_sorted_and_lowercase() {
        assert!(
            STOP_WORDS.windows(2).all(|w| w[0] < w[1]),
            "unsorted near {:?}",
            STOP_WORDS.windows(2).find(|w| w[0] >= w[1])
        );
        assert!(
            STOP_WORDS
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase()))
        );
    }

    #[test]
    fn words_are_lowercased_and_filtered() {
        let found: Vec<String> =
            words("The Ferry's timetable: 2004, at 10:30 — über-café https://x.org ok").collect();
        assert_eq!(found, ["ferry", "timetable", "über", "café"]);
    }

    #[test]
    fn a_groups_distinctive_word_ranks_above_common_words() {
        let mut ferries = GroupWords::default();
        ferries.add_note(
            "The ferry leaves the harbour at noon. Weather today was fine, ferry was late.",
        );
        ferries.add_note("Booked the ferry crossing for today. Harbour busy. Weather grey.");
        ferries.add_note("Ferry timetable changed today; the weather report says wind.");
        let mut dreams = GroupWords::default();
        dreams.add_note("Dreamt I was flying today. The weather in the dream was strange.");
        dreams
            .add_note("Another dream about flying over water; woke up early today, weather calm.");
        let mut work = GroupWords::default();
        work.add_note("Budget meeting today, the weather kept everyone inside.");
        work.add_note("Meeting notes: budget approved today despite the weather.");
        let mut garden = GroupWords::default();
        garden.add_note("Planted tomatoes today; good weather for the garden.");
        garden.add_note("Garden needs water, the weather today is dry. Tomatoes growing.");
        let top = distinctive(&[ferries, dreams, work, garden], usize::MAX);
        // "today" and "weather" are in every note of every group: they come after the
        // group's own words, though each is more frequent in it than "harbour".
        let rank = |g: usize, w: &str| top[g].iter().position(|x| x == w).unwrap();
        assert_eq!(top[0][..2], ["ferry", "harbour"]);
        for common in ["today", "weather"] {
            assert!(rank(0, "harbour") < rank(0, common));
            assert!(rank(1, "dream") < rank(1, common) && rank(1, "flying") < rank(1, common));
            assert!(rank(2, "budget") < rank(2, common) && rank(2, "meeting") < rank(2, common));
            assert!(rank(3, "garden") < rank(3, common) && rank(3, "tomatoes") < rank(3, common));
        }
        assert_eq!(
            distinctive(&[GroupWords::default()], 3),
            [Vec::<String>::new()]
        );
    }

    #[test]
    fn one_long_note_does_not_fill_the_list() {
        let mut g = GroupWords::default();
        g.add_note("garden garden");
        g.add_note("garden zebra zebra zebra zebra zebra zebra");
        let mut other = GroupWords::default();
        other.add_note("kitchen");
        let top = distinctive(&[g, other], 2);
        assert_eq!(top[0], ["garden", "zebra"]);
        assert_eq!(top[1], ["kitchen"]);
    }
}
