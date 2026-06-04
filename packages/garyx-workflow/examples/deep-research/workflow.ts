import {
  defineWorkflow,
  s,
  type JsonValue,
  type WorkflowPhaseInput,
} from "@garyx/workflow";

type ResearchInput = {
  question: string;
  limits: ResearchLimits;
};

type ResearchLimits = {
  maxFetch: number;
  maxVerifyClaims: number;
  votesPerClaim: number;
  refutationsRequired: number;
  searchConcurrency: number;
  fetchConcurrency: number;
  verifyConcurrency: number;
};

type ScopeResult = {
  question: string;
  summary: string;
  angles: SearchAngle[];
};

type SearchAngle = {
  label: string;
  query: string;
  rationale?: string;
};

type SearchResult = {
  results: SearchSource[];
};

type SearchSource = {
  url: string;
  title: string;
  snippet?: string;
  relevance: "high" | "medium" | "low";
};

type ExtractResult = {
  sourceQuality: SourceQuality;
  publishDate?: string;
  claims: ExtractedClaim[];
};

type SourceQuality = "primary" | "secondary" | "blog" | "forum" | "unreliable";

type ExtractedClaim = {
  claim: string;
  quote: string;
  importance: "central" | "supporting" | "tangential";
};

type SourcedClaim = ExtractedClaim & {
  sourceUrl: string;
  sourceTitle: string;
  sourceQuality: SourceQuality;
  publishDate?: string;
  angle: string;
};

type Verdict = {
  refuted: boolean;
  evidence: string;
  confidence: "high" | "medium" | "low";
  counterSource?: string;
};

type VotedClaim = SourcedClaim & {
  verdicts: Verdict[];
  refutedVotes: number;
  survives: boolean;
};

type Report = {
  summary: string;
  findings: Array<{
    claim: string;
    confidence: "high" | "medium" | "low";
    sources: string[];
    evidence: string;
    vote?: string;
  }>;
  caveats: string;
  openQuestions?: string[];
};

type SourceRecord = {
  url: string;
  title: string;
  angle: string;
  sourceQuality: SourceQuality;
  publishDate?: string;
  claims: SourcedClaim[];
};

const PHASES: WorkflowPhaseInput[] = [
  { id: "scope", title: "Scope", detail: "Decompose the question into complementary search angles." },
  { id: "search", title: "Search", detail: "Run parallel web-search agents, one per angle." },
  { id: "fetch", title: "Fetch", detail: "Deduplicate URLs, fetch sources, and extract falsifiable claims." },
  { id: "verify", title: "Verify", detail: "Adversarially verify each high-value claim with independent votes." },
  { id: "synthesize", title: "Synthesize", detail: "Merge survivors, rank by confidence, and cite sources." },
];

const DEFAULT_LIMITS: ResearchLimits = {
  maxFetch: integerEnv("GARYX_DEEP_RESEARCH_MAX_FETCH", 15),
  maxVerifyClaims: integerEnv("GARYX_DEEP_RESEARCH_MAX_VERIFY_CLAIMS", 25),
  votesPerClaim: integerEnv("GARYX_DEEP_RESEARCH_VOTES_PER_CLAIM", 3),
  refutationsRequired: integerEnv("GARYX_DEEP_RESEARCH_REFUTATIONS_REQUIRED", 2),
  searchConcurrency: integerEnv("GARYX_DEEP_RESEARCH_SEARCH_CONCURRENCY", 5),
  fetchConcurrency: integerEnv("GARYX_DEEP_RESEARCH_FETCH_CONCURRENCY", 5),
  verifyConcurrency: integerEnv("GARYX_DEEP_RESEARCH_VERIFY_CONCURRENCY", 8),
};

const SMOKE_LIMITS: ResearchLimits = {
  maxFetch: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_MAX_FETCH", 2),
  maxVerifyClaims: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_MAX_VERIFY_CLAIMS", 2),
  votesPerClaim: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_VOTES_PER_CLAIM", 2),
  refutationsRequired: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_REFUTATIONS_REQUIRED", 2),
  searchConcurrency: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_SEARCH_CONCURRENCY", 2),
  fetchConcurrency: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_FETCH_CONCURRENCY", 2),
  verifyConcurrency: integerEnv("GARYX_DEEP_RESEARCH_SMOKE_VERIFY_CONCURRENCY", 4),
};

const ScopeSchema = s.object<ScopeResult>(
  {
    question: s.string(),
    summary: s.string(),
    angles: s.array(
      s.object(
        {
          label: s.string(),
          query: s.string(),
          rationale: s.string(),
        },
        ["label", "query"],
      ),
      { minItems: 3, maxItems: 6 },
    ),
  },
  ["question", "summary", "angles"],
);

const SearchSchema = s.object<SearchResult>({
  results: s.array(
    s.object<SearchSource>(
      {
        url: s.string(),
        title: s.string(),
        snippet: s.string(),
        relevance: s.enum(["high", "medium", "low"] as const),
      },
      ["url", "title", "relevance"],
    ),
    { maxItems: 6 },
  ),
});

const ExtractSchema = s.object<ExtractResult>({
  sourceQuality: s.enum(["primary", "secondary", "blog", "forum", "unreliable"] as const),
  publishDate: s.string(),
  claims: s.array(
    s.object<ExtractedClaim>({
      claim: s.string(),
      quote: s.string(),
      importance: s.enum(["central", "supporting", "tangential"] as const),
    }),
    { maxItems: 5 },
  ),
});

const VerdictSchema = s.object<Verdict>(
  {
    refuted: s.boolean(),
    evidence: s.string(),
    confidence: s.enum(["high", "medium", "low"] as const),
    counterSource: s.string(),
  },
  ["refuted", "evidence", "confidence"],
);

const ReportSchema = s.object<Report>(
  {
    summary: s.string(),
    findings: s.array(
      s.object({
        claim: s.string(),
        confidence: s.enum(["high", "medium", "low"] as const),
        sources: s.array(s.string()),
        evidence: s.string(),
        vote: s.string(),
      }),
    ),
    caveats: s.string(),
    openQuestions: s.array(s.string()),
  },
  ["summary", "findings", "caveats"],
);

await defineWorkflow({
  name: "Deep Research",
  description: "Fan out web searches, fetch sources, adversarially verify claims, and synthesize a cited report.",
  agent: defaultChildAgent(),
  output: (result: { summary?: string }) => result.summary,
  phases: PHASES,
  async run(flow) {
    const input = normalizeInput(flow.ctx.input);
    await flow.log("deep_research.config", {
      question: input.question,
      childAgentId: defaultChildAgent(),
      limits: input.limits,
    });

    const scope = await flow.phase("Scope").start().agent<ScopeResult>("scope", scopePrompt(input.question), {
      schema: ScopeSchema,
    });
    if (!scope) {
      return failedReport(input.question, "Scope did not return a research plan.");
    }

    flow.logLine(
      `Question scoped into ${scope.angles.length} angles: ${scope.angles.map((angle) => angle.label).join(", ")}`,
    );

    const dedupe = createSourceDedupe(input.limits.maxFetch);
    const searchPhase = flow.phase("Search");
    const fetchPhase = flow.phase("Fetch");

    const sourceBatches = await searchPhase.pipeline(
      scope.angles,
      async (angle) => {
        const search = await searchPhase.agent<SearchResult>(`search:${angle.label}`, searchPrompt(input.question, angle), {
          schema: SearchSchema,
          optional: true,
        });
        if (!search) {
          await flow.log("deep_research.search_empty", { angle: angle.label });
          return null;
        }
        await flow.log("deep_research.search_results", {
          angle: angle.label,
          results: search.results.length,
        });
        return { angle, results: search.results };
      },
      async (search) => {
        if (!search) {
          return [];
        }
        const novel = dedupe.claim(search.results, search.angle.label);
        if (!novel.length) {
          return [];
        }
        return fetchPhase.parallel(
          novel.map((source) => async () => {
            const host = hostName(source.url);
            try {
              const extracted = await fetchPhase.agent<ExtractResult>(
                `fetch:${host}`,
                fetchPrompt(input.question, source, search.angle),
                {
                  schema: ExtractSchema,
                  optional: true,
                },
              );
              if (!extracted) {
                return unreliableSource(source, search.angle.label);
              }
              return toSourceRecord(source, search.angle.label, extracted);
            } catch (error) {
              await flow.log("deep_research.fetch_failed", {
                url: source.url,
                error: errorMessage(error),
              });
              return unreliableSource(source, search.angle.label);
            }
          }),
          { concurrency: input.limits.fetchConcurrency },
        );
      },
    );

    const sources = sourceBatches.flat().filter(isSourceRecord);
    const claims = rankClaims(sources.flatMap((source) => source.claims)).slice(0, input.limits.maxVerifyClaims);

    await flow.log("deep_research.claim_pool", {
      sourcesFetched: sources.length,
      claimsExtracted: sources.reduce((total, source) => total + source.claims.length, 0),
      claimsSelected: claims.length,
      duplicateUrls: dedupe.duplicates.length,
      budgetDropped: dedupe.budgetDropped.length,
    });

    if (!claims.length) {
      return {
        question: input.question,
        summary: `No falsifiable claims were extracted from ${sources.length} fetched sources.`,
        findings: [],
        refuted: [],
        sources: sourcesForResult(sources),
        caveats: "The run found sources but could not extract checkable claims, or all source fetches failed.",
        openQuestions: ["Try a narrower question with named entities, dates, or decision criteria."],
        stats: stats(scope, sources, 0, 0, 0, 0, dedupe),
      };
    }

    const verifyPhase = flow.phase("Verify").start();
    const voted = (
      await verifyPhase.parallel(
        claims.map((claim) => async () => {
          const verdicts = await verifyPhase.parallel(
            Array.from({ length: input.limits.votesPerClaim }, (_, index) => async () =>
              verifyPhase.agent<Verdict>(`v${index + 1}:${claim.claim.slice(0, 42)}`, verifyPrompt(input.question, claim, index, input.limits), {
                schema: VerdictSchema,
                optional: true,
              }),
            ),
            { concurrency: input.limits.votesPerClaim },
          );
          return adjudicateClaim(claim, verdicts.filter(Boolean) as Verdict[], input.limits);
        }),
        { concurrency: input.limits.verifyConcurrency },
      )
    ).filter(Boolean);

    const confirmed = voted.filter((claim) => claim.survives);
    const killed = voted.filter((claim) => !claim.survives);

    await flow.log("deep_research.verify_done", {
      verified: voted.length,
      confirmed: confirmed.length,
      killed: killed.length,
    });

    if (!confirmed.length) {
      return {
        question: input.question,
        summary: `All ${voted.length} checked claims failed adversarial verification.`,
        findings: [],
        refuted: refutedForResult(killed),
        sources: sourcesForResult(sources),
        caveats: "Research is inconclusive because the candidate claims were refuted or did not reach a vote quorum.",
        openQuestions: ["Run with a narrower question or stronger source constraints."],
        stats: stats(scope, sources, claims.length, voted.length, 0, killed.length, dedupe),
      };
    }

    const report = await flow.phase("Synthesize").start().agent<Report>("synthesize", synthesizePrompt(input.question, confirmed, killed), {
      schema: ReportSchema,
    });

    if (!report) {
      return {
        question: input.question,
        summary: `Synthesis failed, returning ${confirmed.length} verified claims unmerged.`,
        findings: confirmed.map((claim) => ({
          claim: claim.claim,
          confidence: bestConfidence(claim.verdicts),
          sources: [claim.sourceUrl],
          evidence: claim.quote,
          vote: voteText(claim),
        })),
        refuted: refutedForResult(killed),
        sources: sourcesForResult(sources),
        caveats: "The synthesis agent failed or was skipped, so findings are not semantically merged.",
        openQuestions: [],
        stats: stats(scope, sources, claims.length, voted.length, confirmed.length, killed.length, dedupe),
      };
    }

    return {
      question: input.question,
      ...report,
      refuted: refutedForResult(killed),
      sources: sourcesForResult(sources),
      stats: {
        ...stats(scope, sources, claims.length, voted.length, confirmed.length, killed.length, dedupe),
        afterSynthesis: report.findings.length,
        agentCalls:
          1 + scope.angles.length + sources.length + voted.length * input.limits.votesPerClaim + 1,
      },
    };
  },
});

function scopePrompt(question: string): string {
  return [
    "You are the scope planner for a deep research workflow.",
    "",
    "Decompose the research question into five complementary web-search angles.",
    "Choose angles that fit the domain. Useful patterns include primary sources, recent news, technical evidence, skeptical or contrarian evidence, practitioner experience, benchmarks, risks, and costs.",
    "Avoid redundant queries. Each query should be specific enough to surface high-signal sources.",
    "",
    "Return the original or lightly normalized question, a short strategy summary, and 3 to 6 angles.",
    "Use submit_result with the fields in the schema.",
    "",
    `Research question: ${question}`,
  ].join("\n");
}

function searchPrompt(question: string, angle: SearchAngle): string {
  return [
    `## Web searcher: ${angle.label}`,
    "",
    `Research question: ${question}`,
    `Search angle: ${angle.label}`,
    `Search query: ${angle.query}`,
    angle.rationale ? `Why this angle matters: ${angle.rationale}` : "",
    "",
    "Use WebSearch. You may refine the query if it improves source quality.",
    "Return the 4 to 6 most relevant sources.",
    "Rank by relevance to the original research question, not just keyword overlap with the query.",
    "Prefer primary or authoritative sources. Skip obvious SEO spam, shallow listicles, content farms, and irrelevant marketing pages.",
    "Include a short snippet explaining why each result matters.",
    "",
    "Use submit_result with the fields in the schema.",
  ]
    .filter(Boolean)
    .join("\n");
}

function fetchPrompt(question: string, source: SearchSource, angle: SearchAngle): string {
  return [
    "## Source extractor",
    "",
    `Research question: ${question}`,
    `Source title: ${source.title}`,
    `Source URL: ${source.url}`,
    `Found through angle: ${angle.label}`,
    source.snippet ? `Search snippet: ${source.snippet}` : "",
    "",
    "Use WebFetch to inspect the page.",
    "Classify source quality as primary, secondary, blog, forum, or unreliable.",
    "Extract 2 to 5 falsifiable claims that bear directly on the research question.",
    "Each claim must be concrete, checkable, and supported by a direct quote from the source.",
    "Rate each claim as central, supporting, or tangential.",
    "Include publish date if it is visible.",
    "If the page is unavailable, irrelevant, paywalled, or too weak, return an empty claims array and sourceQuality unreliable.",
    "",
    "Use submit_result with the fields in the schema.",
  ]
    .filter(Boolean)
    .join("\n");
}

function verifyPrompt(question: string, claim: SourcedClaim, voteIndex: number, limits: ResearchLimits): string {
  return [
    `## Adversarial claim verifier (${voteIndex + 1}/${limits.votesPerClaim})`,
    "",
    "Your job is to try to refute the claim, not to rubber-stamp it.",
    `${limits.refutationsRequired}/${limits.votesPerClaim} refuting votes are enough to kill a claim.`,
    "",
    `Research question: ${question}`,
    "",
    `Claim: ${claim.claim}`,
    `Source: ${claim.sourceUrl} (${claim.sourceQuality})`,
    `Supporting quote: "${claim.quote}"`,
    claim.publishDate ? `Publish date: ${claim.publishDate}` : "",
    "",
    "Checklist:",
    "1. Does the quote actually support the claim, or is the claim an overreach?",
    "2. Search for credible contradicting or qualifying evidence.",
    "3. Is the source quality strong enough for the claim?",
    "4. Is the claim outdated for a fast-moving topic?",
    "5. Is this marketing, cherry-picked benchmark data, speculation, or forum noise?",
    "",
    "Set refuted=true when the claim is unsupported, contradicted, outdated, too weakly sourced, or uncertain.",
    "Set refuted=false only when the claim is current, well-supported, and source quality matches claim strength.",
    "Evidence must be specific. Include a counterSource when you find one.",
    "",
    "Use submit_result with the fields in the schema.",
  ]
    .filter(Boolean)
    .join("\n");
}

function synthesizePrompt(question: string, confirmed: VotedClaim[], killed: VotedClaim[]): string {
  return [
    "## Synthesis agent",
    "",
    `Research question: ${question}`,
    "",
    `${confirmed.length} claims survived adversarial verification.`,
    "Merge claims that say the same thing. Group related claims into findings that directly answer the research question.",
    "Assign confidence per finding: high for multiple strong sources and strong votes, medium for secondary sources or partial votes, low for single-source or weaker evidence.",
    "Write a concise executive summary. Cite sources by URL. State caveats and open questions.",
    "",
    "## Confirmed claims",
    confirmed.map((claim, index) => confirmedClaimBlock(claim, index)).join("\n\n"),
    "",
    killed.length
      ? ["## Refuted claims", killed.map((claim) => `- ${claim.claim} (${claim.sourceUrl}, vote ${voteText(claim)})`).join("\n")].join("\n")
      : "",
    "",
    "Use submit_result with the fields in the schema.",
  ]
    .filter(Boolean)
    .join("\n");
}

function confirmedClaimBlock(claim: VotedClaim, index: number): string {
  const best = claim.verdicts
    .filter((verdict) => !verdict.refuted)
    .sort((left, right) => confidenceRank(left.confidence) - confidenceRank(right.confidence))[0];
  return [
    `### [${index + 1}] ${claim.claim}`,
    `Vote: ${voteText(claim)}`,
    `Source: ${claim.sourceUrl} (${claim.sourceQuality})`,
    `Quote: "${claim.quote}"`,
    best ? `Verifier evidence (${best.confidence}): ${best.evidence}` : "Verifier evidence: no supporting vote details available.",
  ].join("\n");
}

function normalizeInput(raw: JsonValue): ResearchInput {
  const input = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, JsonValue>) : {};
  const question =
    typeof raw === "string"
      ? raw.trim()
      : stringValue(input.question) || stringValue(input.query) || stringValue(input.input);
  if (!question) {
    throw new Error("deep-research requires a research question");
  }
  const mode = stringValue(input.mode) || process.env.GARYX_DEEP_RESEARCH_MODE || "full";
  const base = mode === "smoke" ? SMOKE_LIMITS : DEFAULT_LIMITS;
  return {
    question,
    limits: {
      maxFetch: positiveInteger(input.maxFetch, base.maxFetch),
      maxVerifyClaims: positiveInteger(input.maxVerifyClaims, base.maxVerifyClaims),
      votesPerClaim: positiveInteger(input.votesPerClaim, base.votesPerClaim),
      refutationsRequired: positiveInteger(input.refutationsRequired, base.refutationsRequired),
      searchConcurrency: positiveInteger(input.searchConcurrency, base.searchConcurrency),
      fetchConcurrency: positiveInteger(input.fetchConcurrency, base.fetchConcurrency),
      verifyConcurrency: positiveInteger(input.verifyConcurrency, base.verifyConcurrency),
    },
  };
}

function createSourceDedupe(maxFetch: number) {
  const seen = new Map<string, { angle: string; title: string }>();
  return {
    duplicates: [] as Array<SearchSource & { angle: string; duplicateOf: { angle: string; title: string } }>,
    budgetDropped: [] as Array<SearchSource & { angle: string }>,
    claim(results: SearchSource[], angle: string): SearchSource[] {
      const selected: SearchSource[] = [];
      const sorted = [...results].sort((left, right) => relevanceRank(left.relevance) - relevanceRank(right.relevance));
      for (const source of sorted) {
        const key = normalizedUrl(source.url);
        if (!key) {
          continue;
        }
        const duplicateOf = seen.get(key);
        if (duplicateOf) {
          this.duplicates.push({ ...source, angle, duplicateOf });
          continue;
        }
        if (seen.size >= maxFetch) {
          this.budgetDropped.push({ ...source, angle });
          continue;
        }
        seen.set(key, { angle, title: source.title });
        selected.push(source);
      }
      return selected;
    },
  };
}

function toSourceRecord(source: SearchSource, angle: string, extracted: ExtractResult): SourceRecord {
  return {
    url: source.url,
    title: source.title,
    angle,
    sourceQuality: extracted.sourceQuality,
    publishDate: extracted.publishDate,
    claims: extracted.claims.map((claim) => ({
      ...claim,
      sourceUrl: source.url,
      sourceTitle: source.title,
      sourceQuality: extracted.sourceQuality,
      publishDate: extracted.publishDate,
      angle,
    })),
  };
}

function unreliableSource(source: SearchSource, angle: string): SourceRecord {
  return {
    url: source.url,
    title: source.title,
    angle,
    sourceQuality: "unreliable",
    claims: [],
  };
}

function rankClaims(claims: SourcedClaim[]): SourcedClaim[] {
  return [...claims].sort(
    (left, right) =>
      importanceRank(left.importance) - importanceRank(right.importance) ||
      qualityRank(left.sourceQuality) - qualityRank(right.sourceQuality),
  );
}

function adjudicateClaim(claim: SourcedClaim, verdicts: Verdict[], limits: ResearchLimits): VotedClaim {
  const refutedVotes = verdicts.filter((verdict) => verdict.refuted).length;
  const survives = verdicts.length >= limits.refutationsRequired && refutedVotes < limits.refutationsRequired;
  return {
    ...claim,
    verdicts,
    refutedVotes,
    survives,
  };
}

function failedReport(question: string, summary: string) {
  return {
    question,
    summary,
    findings: [],
    refuted: [],
    sources: [],
    caveats: summary,
    openQuestions: [],
    stats: { angles: 0, sourcesFetched: 0, claimsExtracted: 0, claimsVerified: 0, confirmed: 0, killed: 0 },
  };
}

function sourcesForResult(sources: SourceRecord[]) {
  return sources.map((source) => ({
    url: source.url,
    title: source.title,
    angle: source.angle,
    quality: source.sourceQuality,
    publishDate: source.publishDate,
    claimCount: source.claims.length,
  }));
}

function refutedForResult(claims: VotedClaim[]) {
  return claims.map((claim) => ({
    claim: claim.claim,
    source: claim.sourceUrl,
    vote: voteText(claim),
    evidence: claim.verdicts.find((verdict) => verdict.refuted)?.evidence,
  }));
}

function stats(
  scope: ScopeResult,
  sources: SourceRecord[],
  claimsSelected: number,
  verified: number,
  confirmed: number,
  killed: number,
  dedupe: ReturnType<typeof createSourceDedupe>,
) {
  return {
    angles: scope.angles.length,
    sourcesFetched: sources.length,
    claimsExtracted: sources.reduce((total, source) => total + source.claims.length, 0),
    claimsVerified: verified,
    claimsSelected,
    confirmed,
    killed,
    urlDupes: dedupe.duplicates.length,
    budgetDropped: dedupe.budgetDropped.length,
  };
}

function voteText(claim: VotedClaim): string {
  return `${claim.verdicts.length - claim.refutedVotes}-${claim.refutedVotes}`;
}

function bestConfidence(verdicts: Verdict[]): "high" | "medium" | "low" {
  return (
    verdicts
      .filter((verdict) => !verdict.refuted)
      .sort((left, right) => confidenceRank(left.confidence) - confidenceRank(right.confidence))[0]?.confidence ?? "low"
  );
}

function defaultChildAgent(): string {
  return process.env.GARYX_DEEP_RESEARCH_AGENT_ID || "claude";
}

function isSourceRecord(value: unknown): value is SourceRecord {
  return Boolean(value && typeof value === "object" && "url" in value && "sourceQuality" in value && "claims" in value);
}

function normalizedUrl(value: string): string {
  try {
    const url = new URL(value);
    return `${url.hostname.replace(/^www\./, "")}${url.pathname.replace(/\/$/, "")}`.toLowerCase();
  } catch {
    return value.trim().replace(/#.*$/, "").replace(/\/+$/, "").toLowerCase();
  }
}

function hostName(value: string): string {
  try {
    return new URL(value).hostname.replace(/^www\./, "");
  } catch {
    return "unknown";
  }
}

function relevanceRank(value: SearchSource["relevance"]): number {
  return value === "high" ? 0 : value === "medium" ? 1 : 2;
}

function importanceRank(value: ExtractedClaim["importance"]): number {
  return value === "central" ? 0 : value === "supporting" ? 1 : 2;
}

function qualityRank(value: SourceQuality): number {
  return { primary: 0, secondary: 1, blog: 2, forum: 3, unreliable: 4 }[value];
}

function confidenceRank(value: Verdict["confidence"]): number {
  return value === "high" ? 0 : value === "medium" ? 1 : 2;
}

function stringValue(value: JsonValue | undefined): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function positiveInteger(value: JsonValue | undefined, fallback: number): number {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : fallback;
}

function integerEnv(name: string, fallback: number): number {
  const value = Number.parseInt(process.env[name] ?? "", 10);
  return Number.isInteger(value) && value > 0 ? value : fallback;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
