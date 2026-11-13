This vault is one encrypted file. A few things to know before you fill it.

> [!warning] There is no passphrase recovery
> Without the passphrase the notes are gone. Keep your own backup of the `.hush` file.

## Before a large import

- [ ] Choose the model in Settings → Model.
- [ ] Run *Preview import* on the folder: it shows what would happen without changing the vault.

Hush indexes every note with a local model. Search by meaning, related notes and tag suggestions depend on it. Switching model later ==indexes every note again==: at roughly 2.5 notes a second, 10,000 notes take over an hour.

- **BGE small English v1.5 (int8)**, the default: notes in English.
- **Multilingual E5 small (int8)**: notes in another language, or in several.
- **MiniLM L6 v2, fast English (int8)**: English, about twice as fast. For a very large vault.
- **BGE small English v1.5 (fp16, GPU)**: English, on a machine with a usable GPU.

All but the default are a one-time download.

## Writing

Notes are Markdown. Ctrl+E switches between Source and Live, to see how this note is written.

- `[[Note title]]` links to a note. `#tag` tags it.
- `==text==` is ==highlighted==. `H~2~O` is H~2~O and `x^2^` is x^2^.
- `- [ ]` starts a task, as in the list above. Tick it in Live.
- A quote that starts with `[!note]`, `[!tip]` or `[!warning]` is a callout, like the one at the top.
- Code in a fence that names its language is coloured:

```python
def greet(name):
    return f"Hello, {name}"  # a comment
```

Ctrl+N new note, Ctrl+K search, Ctrl+L lock. Deleted notes go to the trash (Settings → Trash).

Delete this note when you no longer need it.
