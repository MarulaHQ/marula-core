---
bump: minor
---

### Added

- Record freight, duty, brokerage, insurance and handling as cost layers on a receipt. Each layer splits across the receipt's lines by value, quantity or weight, locks its own exchange rate at the document's date, and either adds to what the receipt is worth or posts to the ticket under its own reporting class. A charge with no ticket to post against stays a classified layer, and the reporting-class rollup is where it is found. A wrong layer is corrected by a reversal, never by an edit.
