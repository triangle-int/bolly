# Video analysis

Nolune core no longer analyses videos (#91). Earlier builds shipped a
`watch_video` tool that downloaded YouTube links with `yt-dlp`, compressed
files with `ffmpeg`, uploaded them to the Gemini Files API, and installed
`yt-dlp` at runtime through `pipx` or a `curl` into `/usr/local/bin`. All of
that is gone:

- The server never installs `yt-dlp`, `ffmpeg`, or any other executable at
  runtime.
- The `GOOGLE_AI` / `gemini` token is retired. Existing values are ignored,
  reported once at startup, and dropped from `config.toml` on the next save.
- YouTube URLs are ordinary links in chat.

## What still works

Video and audio remain ordinary user files and chat attachments. Uploads keep their MIME type,
render in the file viewer, and can be attached to a message when the selected
chat provider accepts that input. Text descriptions already bound to persisted
media memories are untouched.

## Future path

If video analysis returns, it will be a reviewed, installable skill with its
own declared dependencies and provider credentials rather than a core
capability. A skill must not install executables on the host or reuse another
integration's secrets; see the capability boundaries in `docs/security/`.
