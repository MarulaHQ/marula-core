---
bump: minor
---

### Added

- Consuming a part for a job now puts its cost on the ticket in the same commit as the shelf movement. The line is priced from the item's last unit cost and reaches the invoice run. A consumption can be recorded as not billable instead, which moves the shelf and leaves the job's costs unchanged. A billable consumption can be reversed once, putting the quantity back and taking the charge off the job, until its line is invoiced; after that a credit note is the correction.
