import { describe, expect, it } from "vitest";

import type { Param, Profile, State } from "../types";
import {
  curatedFields,
  effectiveValue,
  floorRows,
  groupValues,
  humanLabel,
  paramsByGroup,
  searchParams,
  servicesLine,
  shapeLine,
  sourceCounts,
  startingProfile,
  warningsFor,
} from "./model";

const host = {
  cores: 2,
  ram_mb: 512,
  free_disk_gb: 39,
  docker: "29.9.2",
  gpu: null,
  lines: [],
};

function param(overrides: Partial<Param> & Pick<Param, "name">): Param {
  return {
    group: "topology",
    kind: "string",
    required: false,
    default: null,
    secret: false,
    description: "a lever",
    consequence: "something changes when it moves",
    default_rationale: null,
    requires: [],
    conflicts: [],
    profile_value: null,
    values: [],
    ...overrides,
  };
}

function profile(name: string, fits: boolean, floors = { cores: 4, ram_mb: 768, disk_gb: 40 }): Profile {
  return {
    name,
    title: name,
    description: "a shape",
    floors,
    services: [
      { name: "core-api", role: "api", ports: [8080, 8082], needs: ["postgres"] },
      { name: "worker", role: "worker", ports: [8082], needs: ["postgres", "redis"] },
    ],
    fits,
    missing: fits ? [] : ["40 GB free disk (has 39)"],
    sets: [{ name: "BIND_ADDR", value: "0.0.0.0:8080", secret: false }],
  };
}

function state(overrides: Partial<State> = {}): State {
  const allInOne = profile("all-in-one", true);
  const split = profile("split", false);
  return {
    host,
    profiles: [split, allInOne],
    recommendation: { profile: "all-in-one", why: [], shortfall: null, embedding_fits: false },
    params: [],
    groups: ["topology", "ai"],
    env: [],
    status: { target: "compose", services: [], error: null },
    dir: "/tmp/marula",
    embedding: "",
    retrieval: "",
    param_count: 0,
    ...overrides,
  };
}

describe("floorRows", () => {
  it("names the number and marks the floor the host misses", () => {
    const rows = floorRows(profile("split", false), host);
    expect(rows.map((row) => row.ok)).toEqual([false, false, false]);
    expect(rows[1]?.have).toBe("512 MB");
    expect(rows[1]?.need).toBe("768 MB");
    expect(rows[2]?.have).toBe("39 GB");
  });

  it("passes a host that meets every floor", () => {
    const rows = floorRows(profile("all-in-one", true, { cores: 2, ram_mb: 512, disk_gb: 20 }), host);
    expect(rows.every((row) => row.ok)).toBe(true);
  });
});

describe("startingProfile", () => {
  it("takes the engine's recommendation over the first profile in the list", () => {
    expect(startingProfile(state())).toBe("all-in-one");
  });

  it("falls back to the first profile that fits when nothing is recommended", () => {
    const base = state();
    const withNoRecommendation: State = {
      ...base,
      recommendation: { ...base.recommendation, profile: null },
    };
    expect(startingProfile(withNoRecommendation)).toBe("all-in-one");
  });
});

describe("effectiveValue", () => {
  it("ranks the operator's override above the profile's preset", () => {
    const base = state({
      params: [param({ name: "BIND_ADDR", default: "127.0.0.1:8080" })],
    });
    expect(effectiveValue("BIND_ADDR", base, "all-in-one", {}).source).toBe("profile");
    const overridden = effectiveValue("BIND_ADDR", base, "all-in-one", {
      BIND_ADDR: "0.0.0.0:9090",
    });
    expect(overridden.value).toBe("0.0.0.0:9090");
    expect(overridden.source).toBe("you");
  });

  it("keeps a value an earlier apply wrote instead of reporting a default", () => {
    const base = state({
      params: [param({ name: "MARULA_RETENTION_PROFILE", default: "standard" })],
      env: [
        {
          name: "MARULA_RETENTION_PROFILE",
          group: "lifecycle",
          description: "how long records are kept",
          value: "long",
          secret: false,
          present: true,
          source: "kept",
        },
      ],
    });
    const kept = effectiveValue("MARULA_RETENTION_PROFILE", base, "all-in-one", {});
    expect(kept.value).toBe("long");
    expect(kept.source).toBe("kept");
  });

  it("reports a lever with no value at all as unset", () => {
    const base = state({ params: [param({ name: "S3_BUCKET" })] });
    expect(effectiveValue("S3_BUCKET", base, "all-in-one", {}).source).toBe("unset");
  });
});

describe("warningsFor", () => {
  const embeddingKey = param({
    name: "MARULA_EMBEDDING_API_KEY",
    secret: true,
    requires: ["MARULA_EMBEDDING_PROVIDER"],
    conflicts: ["OLLAMA_BASE_URL"],
  });

  it("asks for the lever a choice depends on", () => {
    const base = state({ params: [embeddingKey] });
    const warnings = warningsFor(embeddingKey, base, "all-in-one", {
      MARULA_EMBEDDING_API_KEY: "sk-test",
    });
    expect(warnings).toHaveLength(1);
    expect(warnings[0]?.kind).toBe("needs");
    expect(warnings[0]?.text).toContain("MARULA_EMBEDDING_PROVIDER");
  });

  it("stops asking once the dependency is set", () => {
    const base = state({
      params: [
        embeddingKey,
        param({ name: "MARULA_EMBEDDING_PROVIDER", default: "openai" }),
      ],
    });
    const warnings = warningsFor(embeddingKey, base, "all-in-one", {
      MARULA_EMBEDDING_API_KEY: "sk-test",
    });
    expect(warnings).toHaveLength(0);
  });

  it("names the conflict when both sides of a pair are set", () => {
    const base = state({
      params: [embeddingKey, param({ name: "OLLAMA_BASE_URL", default: "http://ollama:11434" })],
    });
    const warnings = warningsFor(embeddingKey, base, "all-in-one", {
      MARULA_EMBEDDING_API_KEY: "sk-test",
      MARULA_EMBEDDING_PROVIDER: "openai",
    });
    expect(warnings.some((warning) => warning.kind === "conflicts")).toBe(true);
  });

  it("says nothing about a lever nobody touched", () => {
    const base = state({ params: [embeddingKey] });
    expect(warningsFor(embeddingKey, base, "all-in-one", {})).toHaveLength(0);
  });
});

describe("searchParams", () => {
  const params = [
    param({ name: "MARULA_DB_POOL_MAX", description: "pool size" }),
    param({ name: "S3_BUCKET", description: "the bucket the object store writes" }),
    param({ name: "EMAIL_TRANSPORT", group: "email" }),
  ];

  it("returns the catalogue untouched for an empty query", () => {
    expect(searchParams(params, "   ").map((entry) => entry.name)).toEqual(params.map((entry) => entry.name));
  });

  it("puts a name hit above a prose hit", () => {
    const ranked = searchParams(params, "bucket").map((entry) => entry.name);
    expect(ranked[0]).toBe("S3_BUCKET");
  });

  it("finds a lever by the words in its consequence", () => {
    const ranked = searchParams(params, "object store").map((entry) => entry.name);
    expect(ranked).toContain("S3_BUCKET");
  });

  it("finds a lever by its group", () => {
    const ranked = searchParams(params, "email").map((entry) => entry.name);
    expect(ranked).toContain("EMAIL_TRANSPORT");
  });
});

describe("shapeLine", () => {
  it("gives each shipped shape a sentence a card can hold", () => {
    expect(shapeLine(profile("split", true))).toContain("transaction pooler");
    expect(shapeLine(profile("all-in-one", true))).toContain("One process");
  });

  it("falls back to the descriptor's first sentence for a shape it does not know", () => {
    const custom = { ...profile("custom", true), description: "A third shape. Then more detail." };
    expect(shapeLine(custom)).toBe("A third shape.");
  });
});

describe("servicesLine", () => {
  it("counts one process as one process", () => {
    const single = {
      ...profile("all-in-one", true),
      services: [{ name: "core-api", role: "api", ports: [8080], needs: [] }],
    };
    expect(servicesLine(single)).toContain("1 process");
    expect(servicesLine(single)).not.toContain("1 processes");
  });
});

describe("humanLabel", () => {
  it("turns a lever name into the words a person would use", () => {
    expect(humanLabel("MARULA_DB_POOL_MAX")).toBe("Database pool size");
    expect(humanLabel("WEB_BASE_URL")).toBe("Address the team opens");
  });

  it("falls back to the stripped name rather than showing nothing", () => {
    expect(humanLabel("MARULA_SOMETHING_NEW")).toBe("Something new");
  });
});

describe("curatedFields", () => {
  it("skips a curated lever the descriptor no longer publishes", () => {
    const base = state({ params: [param({ name: "WEB_BASE_URL", group: "topology" })] });
    const fields = curatedFields(base);
    expect(fields.map((field) => field.name)).toEqual(["WEB_BASE_URL"]);
    expect(fields[0]?.label).toBe("The address your team will open");
  });
});

describe("groupValues", () => {
  it("groups values under the descriptor's own subjects, in a stable order", () => {
    const groups = groupValues([
      { name: "SMTP_HOST", group: "email", description: "", value: "smtp", secret: false, present: true, source: "default" },
      { name: "POSTGRES_PASSWORD", group: "infrastructure", description: "", value: "****", secret: true, present: false, source: "generated" },
      { name: "WEB_BASE_URL", group: "topology", description: "", value: "http://x", secret: false, present: true, source: "you" },
    ]);
    expect(groups.map((group) => group.group)).toEqual(["topology", "email", "infrastructure"]);
    expect(groups[2]?.title).toBe("Infrastructure");
    expect(groups[0]?.rows).toHaveLength(1);
  });
});

describe("paramsByGroup", () => {
  it("keeps the catalogue's own grouping", () => {
    const groups = paramsByGroup([
      param({ name: "A", group: "topology" }),
      param({ name: "B", group: "ai" }),
      param({ name: "C", group: "topology" }),
    ]);
    expect(groups.map((group) => group.group)).toEqual(["topology", "ai"]);
    expect(groups[0]?.params.map((entry) => entry.name)).toEqual(["A", "C"]);
  });
});

describe("sourceCounts", () => {
  it("counts what the operator set apart from what the plan generated", () => {
    const counts = sourceCounts([
      { source: "you" },
      { source: "generated" },
      { source: "generated" },
      { source: "kept" },
      { source: "default" },
    ]);
    expect(counts).toEqual({ you: 1, kept: 1, generated: 2, other: 1 });
  });
});
