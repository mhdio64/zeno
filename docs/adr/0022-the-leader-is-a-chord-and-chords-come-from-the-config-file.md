# The leader is a chord, and chords come from the config file

Spec §3 makes the leader rebindable — on macOS `Ctrl+Space` is the system's
input-source shortcut and is swallowed before the terminal sees it, so a user
with more than one input source must be able to move it — and §6 puts
`zeno.toml` where that change belongs. Until now the file's parser read one
table, `[settings]`, and the keymap lived entirely in code: the leader was
`Ctrl+Space` because the source said so.

The decision is two things at once. The leader is **a chord, not a key** —
one entry in the same table every other binding goes in — and a chord is
written the way a person says it, `ctrl+b`, `alt+f2`, `ctrl+space`, parsed by
`Chord` and matched against the **key** rather than the character the layout
produces for it, which is the rule every built-in binding already followed
(ADR 0010). A config file that said `ctrl+s` would otherwise be dead on a
Persian or Russian layout while plain typing kept working.

```toml
[keys]
leader = "ctrl+b"

[[keys.bindings]]
keys = "ctrl+shift+w"
command = "toggle_word_wrap"
```

Three consequences are worth stating:

- **A user binding is consulted before the built-ins.** A config file can
  move a command without unbinding where it was, which is the difference
  between "rebindable" and "additive". A chord that names a command the
  editor does not have, or a chord it cannot parse, is a line that is
  skipped: a typo is not a reason for an editor not to start.
- **The old leader's aliases die with it.** A legacy terminal reports
  `Ctrl+Space` three ways — the control-space key, the NUL byte, and `Ctrl+@`,
  which is the same byte by another name, sometimes with the modifiers lost
  along the way. All three are the leader while the leader is `Ctrl+Space`,
  and none of them is once it is `ctrl+b`: `Ctrl+Space` is then an ordinary
  key, and `Leader d w` on it should not still delete a word.
- **`Command` owns the names, the keymap owns the meaning.** The config
  stores two strings; only `Command::from_name` knows that `go to line`,
  `go-to-line` and `go_to_line` are one command. That keeps the parser from
  growing a second copy of the command list, and it means a command added to
  the editor is bindable the day it exists.

The costs: the keymap is read once at startup, so a live reload of
`zeno.toml` (which spec §6 asks for) is a separate piece of work; and because
user bindings shadow built-ins, a config file can shadow `Ctrl+S` and leave
itself unable to save — deliberately, since the alternative is a config file
that silently fails to bind the one key it named. Themes and a `[keys]`
escape hatch for individual *plain* keys are not part of this: the default
keymap in `keymap.md` remains what every unbound key means.
