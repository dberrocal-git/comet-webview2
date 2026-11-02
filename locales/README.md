# Adding a language

All application labels, tooltips, status messages, and native errors are in
`en.json`. The executable also includes English as a fallback.

1. Copy `en.json` to a new UTF-8 file, such as `fr.json`.
2. Translate the values. Keep every key unchanged.
3. Keep placeholders such as `{status}` unchanged.
4. Put the file in the `locales` folder next to `Comet-WebView2.exe`.
5. Set `"language": "fr"` in `%LOCALAPPDATA%\Comet-WebView2\settings.json`.
6. Restart the application.

Missing translations fall back to English. Invalid or missing language files
also fall back to English. Language codes use lowercase letters and hyphens.
Translation values are rendered as text, not HTML.

The native app and its connection manager use this catalog. The device's own
web interface has its own language settings.
