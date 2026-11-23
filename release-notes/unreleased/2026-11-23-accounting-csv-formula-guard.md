---
bump: patch
---

### Security

- The accountant package's CSV cells now use the same spreadsheet-formula guard as the report exports, so text that begins with `=`, `+`, `-` or `@`, including behind leading whitespace, stays text instead of being evaluated.
