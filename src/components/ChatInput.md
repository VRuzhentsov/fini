# ChatInput

Persistent bottom bar for capturing quest input. Used in [[NewQuestForm]].

## Layout

Fixed to the bottom of the screen. Two rows:

1. **Live transcript** — shown only while recording; fades in/out. Displays partial ASR text or "Listening…"
2. **Chat bar** — text input + send button + mic button

## Events

| Event | Payload | Description |
|---|---|---|
| `submit` | `text: string` | Emitted on form submit with the trimmed input value. Input is cleared after emit. |

## Behaviour

### Text input
- `<textarea>` that auto-grows with content, capped at 6 rows
- Readonly while mic is recording (prevents editing mid-transcription)
- Send button disabled when input is empty
- On submit: emits `submit`, clears the field
- `Enter` submits; `Shift+Enter` inserts a newline

### Mic button (PTT — push to talk) — temporarily hidden

> Hidden until voice is re-enabled. See [[voice.rs]].


- **Hold** → starts ASR via [[useVoiceInput]]
- **Release** → stops ASR; transcript is appended to the current text value
- Uses pointer capture so release is detected even if finger slides off
- If ASR is already running on press, force-stops and restarts
- Errors surface via [[useToast]]

### Mic button states
| State | Visual |
|---|---|
| Idle | Dimmed icon |
| Warming up | Spinning animation |
| Recording | Red icon, pulsing ring |

## Dependencies

| Dep | Role |
|---|---|
| [[useVoiceInput]] | ASR start/stop, transcript, warming state |
| [[useToast]] | Error notifications |
