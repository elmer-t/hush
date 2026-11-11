// A small passphrase strength estimate for the create-vault and change-passphrase forms.
// It runs locally on what is typed and never leaves this module. It estimates how many
// guesses an attacker who tries common passwords, common words, keyboard patterns,
// repeats and years first would need, in the spirit of zxcvbn but with a short
// embedded list instead of zxcvbn's 800 kB of dictionaries.

/** Mirrors `MIN_PASSPHRASE_CHARS` in src-tauri/src/db/mod.rs, which is the rule that counts. */
export const MIN_PASSPHRASE_CHARS = 8;

/** The one line of guidance shown under a new passphrase. */
export const PASSPHRASE_GUIDANCE = "Use four or more random words. At least 8 characters.";

export type StrengthLevel = "empty" | "tooShort" | "weak" | "fair" | "strong";
export type Strength = { level: StrengthLevel; bits: number; warning: string | null };

/** Under this many bits of guessing work a passphrase is weak; from STRONG_BITS on, strong. */
const WEAK_BITS = 40;
const STRONG_BITS = 60;

const WARNINGS = {
  tooShort: `Too short: use at least ${MIN_PASSPHRASE_CHARS} characters.`,
  common: "This is one of the most common passwords.",
  pattern: "Keyboard patterns and sequences are easy to guess.",
  repeat: "Repeated characters or words are easy to guess.",
  words: "Common words and dates are easy to guess. Add more words.",
  other: "Easy to guess. Add more words.",
};

// Passwords near the top of every leaked-password list (letters only: digits and
// symbols around them are stripped before the lookup).
const COMMON_PASSWORDS = new Set(
  (
    "password passw0rd passwort wachtwoord motdepasse contrasena senha qwerty qwertz azerty letmein welcome " +
    "welkom willkommen admin administrator login master dragon monkey football baseball soccer hockey iloveyou " +
    "sunshine princess shadow superman batman trustno starwars whatever freedom hello hallo secret geheim " +
    "michael jennifer jordan hunter ranger buster killer charlie andrew michelle jessica daniel thomas robert " +
    "pepper ginger maggie cookie chocolate computer internet samsung google abc abcd qazwsx zaq asdf zxcv " +
    "access changeme default guest test tester user root toor pass passphrase hush vault notes mustang harley " +
    "yankees cowboys tigger pokemon naruto matrix ninja liverpool chelsea arsenal flower cheese banana orange " +
    "purple silver golden diamond lovely loveme blessed jesus angel baby babygirl iloveu fuckyou asshole"
  ).split(" "),
);

// Very common words: an attacker who combines a few of these needs few guesses. Kept
// short on purpose; words not here are counted character by character.
const COMMON_WORDS = new Set(
  (
    "a i am an and are as at be by do go he if in is it me my no of oh ok on or so to up us we you your the " +
    "this that with for from have not but all one two three four five six love like life live home house " +
    "family friend friends happy lucky magic money music sweet heart dog dogs cat cats baby girl boy man " +
    "woman king queen star sun moon sky blue red green black white pink gold cool good best big little " +
    "new old day night time year god dear mom dad mother father sister brother son daughter kids garden " +
    "summer winter spring autumn fall rain snow fire water earth world city street school work office " +
    "apple banana cherry orange lemon coffee tea beer wine pizza food fish bird horse tiger lion bear wolf " +
    "monkey dragon eagle shark rose lily flower tree forest beach ocean sea island river mountain " +
    "car ship boat train book game play player team ball goal win winner hello welcome secret open " +
    "door key lock safe private name forever always never please thank thanks yes yeah " +
    "het de een en van ik je mijn jij wij zijn huis liefde hond kat zon maan water vuur " +
    "der die das und ich du mein liebe haus hund katze sonne le la les et je mon amour maison chien chat"
  ).split(" "),
);
for (const p of COMMON_PASSWORDS) COMMON_WORDS.add(p);
const LONGEST_WORD = Math.max(...[...COMMON_WORDS].map((w) => w.length));

// Rows and columns of common keyboard layouts, and runs of letters and digits.
const SEQUENCES = [
  "abcdefghijklmnopqrstuvwxyz",
  "01234567890",
  "`1234567890-=",
  "qwertyuiop[]\\",
  "asdfghjkl;'",
  "zxcvbnm,./",
  "qwertzuiopü",
  "yxcvbnm",
  "azertyuiop",
  "qsdfghjklm",
  "wxcvbn",
  "1qaz2wsx3edc4rfv5tgb6yhn7ujm8ik,9ol.0p;/",
  "!qaz@wsx#edc$rfv%tgb^yhn&ujm*ik<(ol>)p:?",
];
const SEQUENCES_BOTH_WAYS = [...SEQUENCES, ...SEQUENCES.map((s) => [...s].reverse().join(""))];

const LEET: Record<string, string> = { "0": "o", "1": "i", "3": "e", "4": "a", "5": "s", "7": "t", "@": "a", $: "s", "!": "i" };

/** Bits for one dictionary word: about a 500-word list. */
const WORD_BITS = 9;
/** Bits for a separator between words: one of a few likely ones. */
const SEPARATORS = [" ", "-", "_", ".", ","];
const SEPARATOR_BITS = 2;
/** Bits for a year from 1900 to 2099. */
const YEAR_BITS = 7;
/** Characters looked at; the rest only add to the character count. */
const MAX_ANALYSED = 100;

type Kind = "word" | "year" | "pattern" | "repeat";
type Step = { from: number; bits: number; kind: Kind | null };

export function passphraseStrength(passphrase: string): Strength {
  const chars = Array.from(passphrase);
  if (chars.length === 0) return { level: "empty", bits: 0, warning: null };

  const lower = chars.map((c) => c.toLowerCase());
  // Past this length a passphrase is strong anyway; the cap keeps typing fast.
  const { bits, kinds } = guess(chars.slice(0, MAX_ANALYSED), Math.log2(charsetSize(chars)), new Map());

  if (chars.length < MIN_PASSPHRASE_CHARS) return { level: "tooShort", bits, warning: WARNINGS.tooShort };
  // Digits and symbols added around a common password do little, so strip them first;
  // inside it they are read as letter substitutions.
  const core = Array.from(lower.join("").replace(/^[^\p{L}]+|[^\p{L}]+$/gu, ""), (c) => LEET[c] ?? c).join("");
  if (core && COMMON_PASSWORDS.has(core)) return { level: "weak", bits: Math.min(bits, WORD_BITS + 7), warning: WARNINGS.common };
  if (bits < WEAK_BITS) {
    const warning = kinds.has("pattern")
      ? WARNINGS.pattern
      : kinds.has("repeat")
        ? WARNINGS.repeat
        : kinds.has("word") || kinds.has("year")
          ? WARNINGS.words
          : WARNINGS.other;
    return { level: "weak", bits, warning };
  }
  return { level: bits < STRONG_BITS ? "fair" : "strong", bits, warning: null };
}

/**
 * The fewest bits of guessing that cover `chars`, split into common words, years,
 * keyboard patterns, repeats and single characters, and which of those it used.
 */
function guess(chars: string[], charBits: number, units: Map<string, number>): { bits: number; kinds: Set<Kind> } {
  const lower = chars.map((c) => c.toLowerCase());
  const plain = lower.map((c) => LEET[c] ?? c);

  // Cheapest way to guess each prefix: best[i] covers chars[0..i).
  const best: Step[] = [{ from: 0, bits: 0, kind: null }];
  for (let i = 1; i <= chars.length; i++) best[i] = { from: i - 1, bits: Infinity, kind: null };
  const offer = (from: number, to: number, bits: number, kind: Kind | null) => {
    const total = best[from].bits + bits;
    if (total < best[to].bits) best[to] = { from, bits: total, kind };
  };
  for (let i = 0; i < chars.length; i++) {
    // best[i] is final here: every step into it starts before i.
    // A space or dash between words is one of a few likely separators.
    offer(i, i + 1, SEPARATORS.includes(chars[i]) ? Math.min(SEPARATOR_BITS, charBits) : charBits, null);
    for (let j = i + 1; j <= Math.min(chars.length, i + LONGEST_WORD); j++) {
      const word = plain.slice(i, j).join("");
      if (COMMON_WORDS.has(word)) {
        const raw = chars.slice(i, j).join("");
        const capitals = raw !== raw.toLowerCase() ? 1 : 0;
        const leet = word !== lower.slice(i, j).join("") ? 1 : 0;
        offer(i, j, WORD_BITS + capitals + leet, "word");
      }
    }
    const four = lower.slice(i, i + 4).join("");
    if (/^(19|20)\d\d$/.test(four)) offer(i, i + 4, YEAR_BITS, "year");
    for (let j = i + 3; j <= chars.length; j++) {
      const run = lower.slice(i, j).join("");
      if (!SEQUENCES_BOTH_WAYS.some((s) => s.includes(run))) break;
      offer(i, j, 6 + Math.log2(j - i), "pattern");
    }
    for (let unit = 1; i + unit * 2 <= chars.length; unit++) {
      let times = 1;
      while (sameRun(lower, i, unit, times + 1)) times++;
      if (times >= 3 || (unit > 1 && times >= 2 && unit * times >= 6)) {
        // The unit costs what guessing it once does; repeating it adds little.
        const key = chars.slice(i, i + unit).join("");
        let once = units.get(key);
        if (once === undefined) units.set(key, (once = guess(chars.slice(i, i + unit), charBits, units).bits));
        offer(i, i + unit * times, once + Math.log2(times) + 1, "repeat");
      }
    }
  }

  const kinds = new Set<Kind>();
  for (let i = chars.length; i > 0; i = best[i].from) if (best[i].kind) kinds.add(best[i].kind!);
  return { bits: best[chars.length].bits, kinds };
}

/** Whether chars[i..] starts with the unit chars[i..i+unit) repeated `times` times. */
function sameRun(chars: string[], i: number, unit: number, times: number): boolean {
  if (i + unit * times > chars.length) return false;
  for (let k = unit; k < unit * times; k++) if (chars[i + k] !== chars[i + (k % unit)]) return false;
  return true;
}

/** How many characters an attacker has to try per position, from the kinds used. */
function charsetSize(chars: string[]): number {
  let size = 0;
  if (chars.some((c) => /[a-z]/.test(c))) size += 26;
  if (chars.some((c) => /[A-Z]/.test(c))) size += 26;
  if (chars.some((c) => /[0-9]/.test(c))) size += 10;
  if (chars.some((c) => c === " ")) size += 1;
  if (chars.some((c) => /[!-/:-@[-`{-~]/.test(c))) size += 33;
  if (chars.some((c) => c > "~")) size += 100;
  return Math.max(size, 10);
}
