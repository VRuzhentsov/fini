# STE: the English Fini uses

Fini writes in ASD-STE100 Simplified Technical English (STE). STE is a
controlled language. It makes text easy to read and gives each word one
meaning. Read this file before you write a name, a comment, a document, a
screen text, a pull request, or an instruction for an agent.

## What STE is

ASD (the Aerospace, Security and Defence Industries Association of Europe)
makes STE and keeps it up to date. The standard has two parts:

- **Part 1, writing rules.** 53 rules in 9 sections: words (section 1),
  multi-word nouns (2), verbs (3), sentences (4), procedural writing (5),
  descriptive writing (6), safety instructions (7), punctuation and word
  count (8), and writing practices (9).
- **Part 2, dictionary.** Approximately 900 approved words. Each approved
  word has one part of speech and one meaning. The dictionary also lists
  words that are not approved, and the approved words to use in their
  place.

Fini follows the edition of January 2025. ASD publishes a new edition
approximately every three years. When a new edition comes out, we read its
list of changes and update this file.

## Get the standard

The standard is free. ASD has the copyright, so the repository does not
contain a copy. Each person gets a copy:

1. Fill in the form on the
   [ASD download page](https://www.asd-europe.org/standards-specifications/simplified-technical-english/download-asd-ste-100).
2. Keep the PDF on your computer, outside the repository.
3. For word searches, make a text copy of the PDF.

## What STE applies to in Fini

STE is written for technical documentation. Fini also applies it to code
and to talk. It applies to:

- Names in code: files, modules, types, functions, commands, and fields.
- Documents in `docs/`, specifications, and agent instructions (skills,
  `CLAUDE.md`, `AGENTS.md`).
- Comments in code.
- Text on the screens of the app.
- Pull request titles and descriptions, and commit messages.
- How agents talk to the person they work with.

## Which words to use

Use a word only when it is one of these (rule 1.1):

1. **An approved word from the dictionary**, used as its part of speech and
   with its meaning (rules 1.2 to 1.4). Example: "connect" is an approved
   verb. "Dial" is not approved, so Fini says "connect".
2. **A technical noun** of our subject field (rules 1.5 to 1.11): a name
   for a thing in Bluetooth, networks, computing, or Fini itself. Examples:
   "scanner", "advertiser", "adapter", "channel".
3. **A technical verb** of our subject field (rules 1.12 and 1.13).
   Examples: "enable", "encrypt", "upload".

`docs/glossary.md` is Fini's list of technical nouns and technical verbs.
STE calls this list the company glossary. A technical noun or verb that is
not in the glossary is not approved yet.

Do not use jargon or invented words as names (rule 1.10). Examples:
"arbiter", "directory" for a list in memory, "backend" for an adapter.
Use one technical noun for one item (rule 1.11). Do not give the same item
a second name in a different file.

When you need a word that is not approved:

1. Look for the word in the dictionary. The dictionary often shows the
   approved word to use in its place.
2. If no approved word gives the meaning, find a technical noun or verb
   that people in Bluetooth, networks, or computing know.
3. Tell the project owner the new term. Give the term, its one meaning,
   and why no approved word gives this meaning.
4. Add the term to `docs/glossary.md` only after you get the approval of
   the project owner.

## Look up a word

The dictionary shows each word as an entry. The case of the entry tells you
the status of the word:

- **UPPERCASE entry: an approved word.** Use it only as the part of speech
  and with the meaning that the entry gives.
- **Lowercase entry: a word that is not approved.** The second column gives
  the approved words to use in its place.
- **No entry: the word is not in the dictionary.** It is not approved.

A word can be approved as one part of speech and not as another. Look at
the part of speech in the entry, not only at the word.

Search the text copy of the dictionary for the entry that starts with the
word, in uppercase and in lowercase.

## Check a name before you write it

Do this check for each new name: a file, a module, a type, a function, a
method, a field, a constant, a command, and a test hook. Do it before you
write the name in the code.

1. Divide the name into its words.
2. Look up each word in the dictionary.
3. Keep a word that has an uppercase entry with the part of speech that the
   name uses.
4. Keep a word that is a term in `docs/glossary.md`.
5. Keep a word of the language or of a library that Fini uses.
6. Replace each other word with the approved word from its entry.
7. If no approved word gives the meaning, stop. Ask the project owner for a
   new glossary term (see "Which words to use").
8. Do the same check for each new comment and each log message.

## How to write sentences

- Write procedures (steps to do) in sentences of 20 words or fewer
  (rule 5.1). Write one instruction in each sentence (rule 5.2). Use the
  command form: "Open the dialog." (rule 5.3).
- Write descriptions in sentences of 25 words or fewer (rule 6.3).
  Give information step by step (rule 6.1).
- Use the active voice (rule 3.6): "The scanner finds the device", not
  "The device is found".
- Use a vertical list for complex text (rule 4.3).
- Do not omit words. Do not use contractions (rule 4.2).
- Use American English spelling (rule 1.14).

## Names in code

- Make names from approved words and glossary terms. Example:
  `identify_device`, `AvailableDevice`, `scan_lock`. Do the check in
  "Check a name before you write it" for each new name.
- Keep the language's own words as they are: Rust, TypeScript, and Vue
  keywords, and names from libraries that Fini uses.
- A wire message name is a technical noun of the Fini protocol. Example:
  `IdentifyDevice`, `DeviceIdentified`.
- New files and new terms need the approval of the project owner before
  you make them.

## Change to STE

Fini started to use STE in October 2026. During the change to STE:

- All new text and code follows STE.
- When you change a file, change its names, comments, and text to STE too.
- Text and code that nobody changes stay as they are until somebody
  changes them.
