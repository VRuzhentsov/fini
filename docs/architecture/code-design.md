# Code design

How Fini code is shaped, in all parts of the app. Read this before you write
or change code. The words are in [`docs/glossary.md`](../glossary.md), and how
to write names is in [`docs/naming.md`](../naming.md).

When this file and the code do not agree, this file is correct and the code
has a defect.

## Objects own their state and their behavior

- Each part of the app is an object: a service, a channel, a repository, a
  scanner. The object owns its state, and its methods are the only code that
  changes that state.
- Write behavior as a method on the object that owns the data that the
  behavior uses.
- Do not write a free function for behavior that reads or changes the state of
  an object. A free function is only for a pure conversion of values that no
  object owns.
- Give an object the things that it needs by DI (see Patterns). An object
  that the state owns gets the state as an argument, and does not keep a
  copy of it.

## Interfaces

- When two or more objects do the same job, they implement one interface: a
  trait in Rust, an interface type in TypeScript.
- A behavior that every implementation has goes on the interface. When the
  behavior is the same for all implementations, write it one time, as a
  default method of the interface. Each implementation gives only what is
  different.
- A platform that does not support a feature implements the same interface
  with methods that do nothing or report that the feature is not there. The
  callers do not change for that platform.
- Callers use the interface, not the type of one implementation, when the
  behavior is the same for all.

## Patterns

Build the app from these five patterns. Each part is one of them, and each
is used through an interface.

- **RPC (commands):** the UI calls the backend through Tauri commands. A
  command gives back one answer.
- **Service:** an object that does one job and owns the state of that job
  (a channel, a scanner, SpaceSync). Other parts call it through its
  interface.
- **Repository:** the only object that reads and writes one kind of stored
  data (the pairs, the channels of a pair, the outbox). A service gets data
  from a repository, and does not use the database directly. How a
  repository gets its connection, and the rules for transactions, are in
  [`docs/db.md`](../db.md).
- **PubSub (event bus):** a service publishes an event when its state
  changes (a device heard, a connection up, a connection down). Other
  services subscribe to the events they need. The publisher does not know
  its subscribers.
- **DI (dependency injection):** an object gets its services and
  repositories in its constructor, as interfaces. The app start makes the
  objects and connects them. A test gives other implementations of the
  same interfaces.

## One answer for one question

- Each question has one place in the code that answers it. Every caller asks
  that place.
- Before you write new code, search for an object, a method, or a constant that
  does the job already. Use it. If it does almost the job, change it so that it
  does the job, and use it.
- When you find two copies of the same behavior, merge them into one, on the
  object or the interface that owns the behavior.

## Code structure

- Put a long condition in a local variable with a name that tells what the
  condition means. Then use the variable in the `if` or the `match`.
  ([rust-analyzer style, "Helper Variables"](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/style.md#helper-variables))

  ```rust
  // GOOD
  let rustfmt_not_installed =
      captured_stderr.contains("not installed") || captured_stderr.contains("not available");
  match output.status.code() {
      Some(1) if !rustfmt_not_installed => Ok(None),
      _ => Err(format_err!("rustfmt failed:\n{}", captured_stderr)),
  };
  // BAD
  match output.status.code() {
      Some(1)
          if !captured_stderr.contains("not installed")
             && !captured_stderr.contains("not available") => Ok(None),
      _ => Err(format_err!("rustfmt failed:\n{}", captured_stderr)),
  };
  ```

- Do the check of a precondition in the caller, and give the function a type
  that shows the result of the check. Keep the `if` and `match` statements in
  the parent function, and keep the helper functions without branches. A
  branch on the state of an object stays in the method of that object (see
  "Objects own their state and their behavior").
  ([rust-analyzer style, "Function Preconditions"](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/style.md#function-preconditions);
  [matklad, "Push Ifs Up"](https://matklad.github.io/2023/11/15/push-ifs-up-and-fors-down.html#Push-Ifs-Up);
  [TigerStyle, "Safety"](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md#safety))

  ```rust
  // GOOD
  fn frobnicate(walrus: Walrus) {
      ...
  }
  // BAD
  fn frobnicate(walrus: Option<Walrus>) {
      let walrus = match walrus {
          Some(it) => it,
          None => return,
      };
      ...
  }
  ```

- When a function checks a value, it returns what it found, and the caller
  uses that value. Do not check in one function and calculate the same value
  again in a different function. This is a value in one call, not stored
  state.
  ([rust-analyzer style, "Function Preconditions"](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/style.md#function-preconditions);
  [Alexis King, "The power of parsing"](https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/#the-power-of-parsing))

  ```rust
  // GOOD
  if let Some(contents) = string_literal_contents(s) {
      ...
  }
  // BAD
  if is_string_literal(s) {
      let contents = &s[1..s.len() - 1];
  }
  ```

- Put each new module in a new file, and write only the `mod` declaration in
  the parent file. Only the `tests` module of a file stays inline in that
  file. Give the new file a name as [`docs/naming.md`](../naming.md) tells
  ("Names", "Case").
  ([The Rust Book, "Separating Modules into Different Files"](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html);
  [The Rust Book, "Unit Tests"](https://doc.rust-lang.org/book/ch11-03-test-organization.html#unit-tests))

  ```rust
  // GOOD: src/lib.rs
  mod front_of_house;
  // BAD: src/lib.rs
  mod front_of_house {
      pub mod hosting {
          pub fn add_to_waitlist() {}
      }
  }
  ```

## Remove what your change makes unused

- When your change makes a function, a method, a field, an import, or a
  re-export unused, remove it in the same change.
- When your change replaces a behavior, remove the old behavior. Do not keep
  two paths.

## Before you write code

1. Find the object that owns the data that the new code uses.
2. Find the interface that this object implements.
3. Search for code that does the same job already.
4. If code that does the job exists, use it or change it.
5. If the behavior is the same for all implementations of the interface, put
   it on the interface.
6. Else, put it on the object that owns the data.
7. Do the name check in `docs/naming.md` for each new name.
8. Remove the code that your change makes unused.

A new file, a new object, or a new interface needs the approval of the project
owner before you make it.
