# Naming and writing

How Fini writes: which English it uses, how to choose each word, and how to
write a name in code. What a word *means* is in
[`glossary.md`](glossary.md).

When a rule here and the code do not agree, this file is correct and the code
has a defect.

## The English Fini uses: STE

Fini writes in ASD-STE100 Simplified Technical English (STE). STE is a
controlled language. It makes text easy to read and gives each word one
meaning.

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
approximately every three years. When a new edition comes out, read its list
of changes and update this file.

### Get the standard

The standard is free. ASD has the copyright, so the repository does not
contain a copy. Each person gets a copy:

1. Fill in the form on the
   [ASD download page](https://www.asd-europe.org/standards-specifications/simplified-technical-english/download-asd-ste-100).
2. Keep the PDF on your computer, outside the repository.
3. For word searches, make a text copy of the PDF.

### What STE applies to

STE is written for technical documentation. Fini also applies it to code and
to talk:

- Names in code: files, modules, types, functions, methods, fields,
  constants, commands, and test hooks.
- Comments in code and log messages.
- Documents in `docs/` and `specs/`, and agent instructions (skills,
  `CLAUDE.md`, `AGENTS.md`).
- Text on the screens of the app.
- Pull request titles and descriptions, and commit messages.
- How agents talk to the person they work with.

### Which words to use

Use a word only when it is one of these (rule 1.1):

1. **An approved word from the dictionary**, used as its part of speech and
   with its meaning (rules 1.2 to 1.4).
2. **A technical noun** of our subject field (rules 1.5 to 1.11): a name for
   a thing in Bluetooth, networks, computing, or Fini itself.
3. **A technical verb** of our subject field (rules 1.12 and 1.13).

[`glossary.md`](glossary.md) is Fini's list of technical nouns and technical
verbs. STE calls this list the company glossary. A technical noun or verb that
is not in the glossary is not approved yet.

Do not use jargon or invented words (rule 1.10). Use one technical noun for
one item (rule 1.11). Do not give the same item a second name in a different
file.

When you need a word that is not approved:

1. Look for the word in the dictionary. Its entry often gives the approved
   word to use in its place.
2. If no approved word gives the meaning, find a technical noun or verb that
   people in Bluetooth, networks, or computing know.
3. Tell the project owner the new term. Give the term, its one meaning, and
   why no approved word gives this meaning.
4. Add the term to the glossary only after you get the approval of the
   project owner.

### Look up a word

The dictionary shows each word as an entry. The case of the entry tells you
the status of the word:

- **UPPERCASE entry: an approved word.** Use it only as the part of speech and
  with the meaning that the entry gives.
- **Lowercase entry: a word that is not approved.** The second column gives
  the approved words to use in its place.
- **No entry: the word is not in the dictionary.** It is not approved.

A word can be approved as one part of speech and not as another. Look at the
part of speech in the entry, not only at the word.

Search the text copy of the dictionary for the entry that starts with the
word, in uppercase and in lowercase. The glossary's list "Words deliberately
not used" gives the words that Fini replaced, with their replacements.

### Check a name before you write it

Do this check for each new name, before you write it in the code:

1. Divide the name into its words.
2. Look up each word in the dictionary.
3. Keep a word that has an uppercase entry with the part of speech that the
   name uses.
4. Keep a word that is a term in the glossary.
5. Keep a word of the language or of a library that Fini uses.
6. Replace each other word with the approved word from its entry.
7. If no approved word gives the meaning, stop. Ask the project owner for a
   new glossary term (see "Which words to use").

Do the same check for each new comment and each log message. Before you
commit, read your diff for the words in the glossary's list "Words
deliberately not used".

### How to write sentences

- Write procedures (steps to do) in sentences of 20 words or fewer
  (rule 5.1). Write one instruction in each sentence (rule 5.2). Use the
  command form (rule 5.3).
- Write descriptions in sentences of 25 words or fewer (rule 6.3). Give
  information step by step (rule 6.1).
- Use the active voice (rule 3.6).
- Use a vertical list for complex text (rule 4.3).
- Do not omit words. Do not use contractions (rule 4.2).
- Use American English spelling (rule 1.14).

### Change to STE

Fini started to use STE in October 2026. During the change to STE:

- All new text and code follows STE.
- When you change a file, change its names, comments, and text to STE too.
- Text and code that nobody changes stay as they are until somebody changes
  them.

## Names in code

Make each name from approved words and glossary terms, after the check
above. Keep the words of the language (Rust, TypeScript, and Vue keywords)
and the names from libraries that Fini uses as they are. A wire message name
is a technical noun of the Fini protocol.

New files and new terms need the approval of the project owner before you
make them.

### Module layout

`services/communication/` holds the three parts of how two devices reach each
other:

| Module | Owns |
|---|---|
| `pairing` | the trust between two devices, and the channels that a pair has |
| `channel` | the connections: the connection code, the framing, and the encryption seam |
| `sync` | the application protocol on a channel, and the outbox |

Give a module the name of the concept that it owns, not the name of the trait
that it exports.

ADRs 0001 to 0006 are older than this layout. They refer to the old paths
`services/{transport,space_sync,device_connection}/`. They are records of the
decisions as they were made, so they stay as they are.

### Plurals

- **A table has a plural name** when a row is one item that you can count. A
  table keeps its name when the name is a mass noun or an activity log.
- **A directory that holds many items has a plural name.**
- **A module for one concept has a singular name.**
- **A folder that groups the parts of a screen has a singular name**: the
  name of the item that the parts belong to.
- **A type has a singular name.** A type for a collection says that it is a
  collection.

### Case

- **Files**: kebab-case where the framework allows it. Vue single-file
  components use `PascalCase.vue`, and Rust modules use `snake_case.rs`, as
  their tools expect.
- **Rust**: `snake_case` items, `PascalCase` types, `SCREAMING_SNAKE_CASE`
  constants.
- **TypeScript**: `camelCase` local names and functions, `PascalCase` types.
  A field that goes across the Tauri bridge keeps the Rust spelling on the two
  sides: one name for each field.
- **DOM hooks**: `data-testid` and `data-*` attributes are kebab-case. They
  give the name of the item, not its position.
- **Tauri commands**: `snake_case`, with the prefix of the part that owns them
  (`space_sync_*`, `device_connection_*`). The prefix `device_connection_*` is
  older than `communication/pairing/`. It is not renamed yet.
