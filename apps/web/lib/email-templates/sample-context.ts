// Sample context for the template editor's preview pane.
//
// One realistic MSP-flavored value per placeholder key, so a preview reads
// like a real message instead of a field of `{{…}}`s. Values mirror what the
// render context actually resolves (`crates/core-db/src/templates.rs`):
// priorities render as `p1..p4`, statuses as the board status name, numbers
// as plain digits — a preview that flatters the data is worse than none.

export const SAMPLE_CONTEXT: Record<string, string> = {
  "org.name": "Northwind IT Services",
  "user.name": "Priya Raman",
  "client.name": "Harborview Dental",
  "contact.name": "Marcus Webb",
  "contact.email": "marcus@harborviewdental.example",
  "contact.phone": "+44 20 7946 0812",
  "ticket.number": "1042",
  "ticket.summary": "Laptop won't connect to the office Wi-Fi",
  "ticket.priority": "p2",
  "ticket.status": "In progress",
  "assignee.name": "Dan Okafor",
  "quote.number": "2044",
  "quote.status": "Draft",
  "quote.total": "4,280.00",
  "quote.currency": "USD",
  "project.name": "Office Wi-Fi refresh",
  "project.status": "On track",
};
