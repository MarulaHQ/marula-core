---
bump: patch
---

### Fixed

- The landed-cost step keeps its draft bound to the receipt it belongs to: a new receipt starts clean, a retried charge is never recorded twice, and a backdated document date is no longer replaced with today.
