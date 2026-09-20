//! One-shot generator for migration 0090's `ai_decision_question_sets`
//! seed rows. Run: `cargo run -p core-decision --example seed_sql`
//! (kept out of the library on purpose — the migration is the artifact).

fn main() {
    for (i, key) in core_decision::starters::SURFACE_KEYS.iter().enumerate() {
        let json = core_decision::starters::starter_set_json(key).unwrap();
        let escaped = json.replace('\'', "''");
        println!(
            "  ('0a200000-0000-0000-0000-{id:012x}', '{key}', 1, '{escaped}'),",
            id = i
        );
    }
}
