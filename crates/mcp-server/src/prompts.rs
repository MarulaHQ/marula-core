//! MCP prompts: reusable workflows, authored in code.
//!
//! A prompt is content, not data: it ships with the release, names the record
//! it starts from, and tells the agent what to read and what to produce. It is
//! listed only when the caller can perform that first read, so a prompt never
//! promises a workflow the session cannot follow.
//!
//! Every prompt takes exactly one argument — the record id — because that is
//! the only thing a template needs to be specific, and it is the same grammar
//! a resource URI accepts ([`crate::resources::is_record_id`]).

pub struct Argument {
    /// The argument name the client passes (`ticket_id`).
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

pub struct Prompt {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// The capability the first read needs (`tickets:read`).
    pub requires: &'static str,
    /// The resource family the prompt starts from, for the embedded link.
    pub resource: &'static str,
    pub argument: Argument,
    /// The instruction text, with `{id}` where the record id goes.
    pub body: &'static str,
}

pub const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "triage_ticket",
        title: "Triage a ticket",
        description: "Read one ticket and propose the next single action, grounded in the record.",
        requires: "tickets:read",
        resource: "tickets",
        argument: Argument {
            name: "ticket_id",
            title: "Ticket",
            description: "The ticket's id.",
        },
        body: "Triage Marula ticket {id}.\n\
               \n\
               Start from the record: read marula://tickets/{id}. Then read what it \
               already carries — comments, checklist and cost lines — with the read tools \
               your session offers.\n\
               \n\
               Report, in this order:\n\
               1. What the ticket is about, in one sentence.\n\
               2. Where it stands: board status, priority, assigned technician, and the \
               client's contract position.\n\
               3. What the record already establishes that a next action would rely on.\n\
               4. The single next action and who owns it.\n\
               \n\
               Cite a record for each claim. Where the record is silent, write \"not \
               recorded\" and name the field you would need, rather than inferring it.",
    },
    Prompt {
        name: "client_qbr_pack",
        title: "Assemble a client QBR pack",
        description: "Build a quarterly business review for one client from its records.",
        requires: "clients:read",
        resource: "clients",
        argument: Argument {
            name: "client_id",
            title: "Client",
            description: "The client's id.",
        },
        body: "Assemble a quarterly business review pack for Marula client {id}.\n\
               \n\
               Start from the record: read marula://clients/{id}. Then gather the period's \
               tickets, documents and contracts with the read tools your session offers, \
               and say plainly which families your grants do not include.\n\
               \n\
               Produce:\n\
               1. The relationship in three lines: who the client is, the contract \
               position, and what changed this quarter.\n\
               2. Ticket volume and mix: opened, resolved, still open, and the recurring \
               themes with the tickets that show them.\n\
               3. Service quality: anything that breached or approached an SLA, with the \
               ticket that shows it.\n\
               4. The three items the account meeting should decide, each with the record \
               behind it.\n\
               \n\
               State the period the numbers cover, and leave a number out rather than \
               estimating it.",
    },
    Prompt {
        name: "document_review",
        title: "Review a document",
        description:
            "Review one document against the record and propose the smallest accurate edits.",
        requires: "documents:read",
        resource: "documents",
        argument: Argument {
            name: "document_id",
            title: "Document",
            description: "The document's id.",
        },
        body: "Review Marula document {id}.\n\
               \n\
               Start from the record: read marula://documents/{id}. Then read its versions \
               with documents_list_versions if your session offers that tool.\n\
               \n\
               Produce:\n\
               1. Purpose and owner, one line each.\n\
               2. What the current version changes against the previous one.\n\
               3. Every claim about a client, contract or commitment that no record \
               supports, quoted and flagged.\n\
               4. The smallest set of edits that would make the document accurate.\n\
               \n\
               Do not rewrite the document. This is a review, and the edits are a proposal \
               for a person to accept.",
    },
];

/// One prompt by name.
pub fn by_name(name: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|prompt| prompt.name == name)
}

/// The prompt body with its record id substituted.
pub fn body_for(prompt: &Prompt, id: &str) -> String {
    prompt.body.replace("{id}", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::FAMILIES;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = PROMPTS.iter().map(|prompt| prompt.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn every_prompt_substitutes_its_record_id() {
        for prompt in PROMPTS {
            let body = body_for(prompt, "018f0a5e-0000-7000-8000-000000000001");
            assert!(
                body.contains("018f0a5e-0000-7000-8000-000000000001"),
                "{} never names the record",
                prompt.name
            );
            assert!(
                !body.contains("{id}"),
                "{} leaves the placeholder",
                prompt.name
            );
        }
    }

    #[test]
    fn every_prompt_starts_from_a_published_family() {
        for prompt in PROMPTS {
            assert!(
                FAMILIES
                    .iter()
                    .any(|family| family.family == prompt.resource),
                "{} names an unpublished family: {}",
                prompt.name,
                prompt.resource
            );
        }
    }

    #[test]
    fn a_prompt_is_found_by_name_or_not_at_all() {
        assert_eq!(by_name("triage_ticket").unwrap().requires, "tickets:read");
        assert!(by_name("not_a_prompt").is_none());
    }
}
