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
- Give an object the things that it needs: in its constructor, or as an
  argument of the call. Do not let an object find them itself (dependency
  injection). An object that the state owns gets the state as an argument, and
  does not keep a copy of it.

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

## One answer for one question

- Each question has one place in the code that answers it. Every caller asks
  that place.
- Before you write new code, search for an object, a method, or a constant that
  does the job already. Use it. If it does almost the job, change it so that it
  does the job, and use it.
- When you find two copies of the same behavior, merge them into one, on the
  object or the interface that owns the behavior.

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
