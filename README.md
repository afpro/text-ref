# text-ref

**Copy, edit and share text without duplicating its storage.**

**English** | [简体中文](README_cn.md)

`text-ref` is a small, dependency-free Rust crate built around two types:

- `TextRef<'a>` — an immutable text snapshot that is cheap to clone. It either
  borrows a `&str` or co-owns an `Arc<String>` buffer.
- `TextMut` — the uniquely owned, mutable counterpart (a thin `String`
  wrapper) used to build text and to apply edits.

Text can be passed around and cloned without copying the bytes, frozen from a
mutable buffer for free, and edited explicitly with copy-on-write semantics:
the text is duplicated only when someone else still holds a snapshot.

## Why another string type?

The standard library gives you three options, each with a trade-off:

- `String` — easy to edit, but every `clone()` allocates and copies the whole
  text.
- `&str` — zero cost, but tied to a borrow and cannot be stored independently.
- `Arc<str>` — cheap to clone and share, but gives no way to edit without
  allocating a fresh buffer.

`text-ref` combines them. A snapshot is either borrowed or shared behind an
`Arc`, so cloning is O(1); when an edit is actually needed, the snapshot is
converted back into an owned `TextMut`, reusing the existing allocation if it
is the last owner.

## Features

- **No dependencies.**
- **O(1) snapshots.** Cloning a `TextRef` copies a pointer or increments a
  reference count — never the text.
- **Zero-copy freezing.** `TextMut::freeze()` moves the `String` buffer into
  the `Arc`; the bytes are not copied.
- **Copy-on-write editing.** `TextRef::edit()` reuses a shared buffer when it
  is the sole owner, and clones the text only when other snapshots are alive.
- **In-place edits.** `TextRef::edit_in_place()` runs a closure with
  `&mut String` and puts the edited text back, returning the closure result.
- **Ergonomic.** `TextMut` dereferences to `String` (`Deref`/`DerefMut`);
  `TextRef` dereferences to `str` and implements `AsRef<str>`, `Display`,
  `Clone` and `Debug`.
- **Documented.** Every public item has rustdoc, and the examples are compiled
  and run as doc tests.

## Installation

The crate is not published on crates.io yet. Add it from Git or as a path
dependency:

```toml
[dependencies]
# From Git
text-ref = { git = "https://github.com/afpro/text-ref" }

# Or from a local checkout
# text-ref = { path = "../text-ref" }
```

Requires Rust 1.85 or newer (the crate uses the 2024 edition).

## Quick start

```rust
use text_ref::{TextMut, TextRef};

fn main() {
    // 1. Build the text while you own it exclusively.
    let mut text = TextMut::with_capacity(64);
    text.push_str("hello");
    text.push_str(", world");

    // 2. Freeze it. The buffer moves into shared storage; no bytes are copied.
    let snapshot: TextRef<'static> = text.freeze();

    // 3. Snapshots clone cheaply - every clone points at the same buffer.
    let shown = snapshot.clone();
    let kept = snapshot.clone();
    println!("{shown}"); // TextRef implements Display

    // 4. Editing is copy-on-write: `kept` and the original still see the old text.
    let mut edited = shown.edit();
    edited.push('!');
    assert_eq!(edited.as_str(), "hello, world!");
    assert_eq!(&*kept, "hello, world");
}
```

Editing in place, with the closure returning a value:

```rust
use text_ref::{TextMut, TextRef};

fn main() {
    let mut snapshot: TextRef<'static> = TextMut::from("hello").freeze();

    let length = snapshot.edit_in_place(|s| {
        *s = s.to_uppercase();
        s.len() // returned to the caller
    });

    assert_eq!(length, 5);
    assert_eq!(&*snapshot, "HELLO");
}
```

## How it works

```text
  TextMut (unique, mutable)  ── freeze() ──►  TextRef<'static> (shared, immutable)
        ▲                                              │
        └────────── edit() / edit_in_place() ──────────┘
```

A `TextRef` is one of two variants:

| Type / variant | Storage | Clone cost | Mutation |
| --- | --- | --- | --- |
| `TextMut` | exclusive `String` | O(n) — copies the text | yes, through `&mut` |
| `TextRef::Ref` | borrowed `&str` | O(1) — copies a fat pointer | no |
| `TextRef::Shared` | `Arc<String>` | O(1) — increments a refcount | no |

The shared variant holds `Arc<String>` rather than `Arc<str>` on purpose: a
`String` is sized, so `Arc::try_unwrap` can hand the buffer back and let it be
edited in place. `Arc<str>` would always require a fresh allocation to mutate.

### Copy-on-write in detail

`TextRef::edit()` consumes the snapshot and produces a `TextMut`:

- `TextRef::Ref` — the borrowed bytes are copied into a new `String`.
- `TextRef::Shared` — `Arc::try_unwrap` is attempted:
  - last owner: the `String` is recovered as-is, no copy, no allocation;
  - other snapshots alive: the text is cloned once, and those snapshots keep
    pointing at the unchanged original.

Because snapshots are immutable, sharing them needs no locks and no
synchronization: readers can only observe the text they were given.

### Panic safety of `edit_in_place`

`edit_in_place` moves the text out of `self` (leaving an empty string behind),
edits the temporary buffer, then freezes it back into `self`. If the closure
panics, `self` is left as an empty `TextRef::Ref("")` and the previous content
is dropped during unwinding.

## API overview

| Item | Description |
| --- | --- |
| `TextMut::new()` / `Default` | Empty text, no allocation. |
| `TextMut::with_capacity(n)` | Empty text with room for at least `n` bytes. |
| `TextMut::from(value)` | Build from anything convertible to `String` (`&str`, `String`, ...). |
| `TextMut::freeze()` | Move the buffer into a `TextRef<'static>` without copying. |
| `TextRef::Ref(&str)` | Construct a borrowed snapshot directly. |
| `TextRef::Shared(Arc<String>)` | Construct a shared snapshot directly. |
| `TextRef::edit()` | Convert into a `TextMut`, copying only if the text is shared. |
| `TextRef::edit_in_place(f)` | Edit through `&mut String` and return the closure result. |

Trait implementations:

| Trait | `TextMut` | `TextRef<'a>` |
| --- | --- | --- |
| `Clone` | deep copy, O(n) | cheap copy, O(1) |
| `Debug` | yes | yes |
| `Deref` | → `String` | → `str` |
| `DerefMut` | → `String` | — |
| `AsRef<str>` | — | yes |
| `Display` | — | yes |

`TextRef` does not implement comparison traits; compare it as a string slice,
for example `&*snapshot == "hello"` or `snapshot.as_ref() == "hello"`.

## Performance

| Operation | Text copied | Allocation |
| --- | --- | --- |
| `TextMut::freeze()` | none | one small `Arc` header |
| `TextRef::clone()` | none | none |
| `TextRef::edit()`, sole owner | none | none |
| `TextRef::edit()`, shared | all of it (O(n)) | one `String` |
| `edit_in_place()`, sole owner | none | none |
| `edit_in_place()`, shared | all of it (O(n)) | one `String` |
| `TextMut::clone()` | all of it (O(n)) | one `String` |

The intended pattern is: build with `TextMut`, freeze once, share widely, and
unfreeze only when an edit is genuinely required.

## Comparison

| Capability | `String` | `Cow<'a, str>` | `Arc<str>` | `TextRef` |
| --- | --- | --- | --- | --- |
| Cheap clone | no | borrowed only | yes | yes |
| Shares one buffer | no | no | yes | yes |
| Holds borrowed text | no | yes | no | yes |
| Edits without copying after sharing | — | — | no | yes, when sole owner |

## Thread safety

`TextRef<'static>` is `Send + Sync`: clones can be moved to other threads and
read concurrently without synchronization, because the underlying text is
immutable. `TextMut` is `Send + Sync` as well, but mutating it requires `&mut`,
so cross-thread editing needs external synchronization (or a per-thread copy).
A `TextRef<'a>` may borrow non-`'static` data; it is still `Send + Sync`, but
it cannot outlive its borrow, so send it to another thread only within that
lifetime (for example with a scoped thread).

## When not to use it

- This is not a rope or a text editor: when the text is shared, every edit
  copies the whole string.
- There is no interior mutability: a shared snapshot cannot be changed in
  place by another thread.
- If you only ever build a string once and read it, a plain `String` or
  `&str` is simpler; the benefit here appears when text is both shared and
  occasionally edited.

## Development

```sh
cargo test          # runs the doc tests (examples included)
cargo clippy
cargo doc --open
```

## License

MIT — see [LICENSE](LICENSE).
