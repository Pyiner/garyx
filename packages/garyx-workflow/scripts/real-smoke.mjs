import { agent, parallel, schema, workflow } from "../dist/index.js";

const fanout = Number.parseInt(process.env.GARYX_WORKFLOW_SMOKE_FANOUT ?? "8", 10);

const StructuredSmoke = schema({
  type: "object",
  additionalProperties: false,
  required: ["summary", "ok"],
  properties: {
    summary: { type: "string" },
    ok: { type: "boolean" },
  },
});

const result = await workflow({
  name: "SDK real workflow smoke",
  description: "Real Garyx workflow SDK structured-result and fanout smoke",
  async run(ctx) {
    await ctx.log("sdk.real_smoke.structured_started", { fanout });
    const finding = await agent(
      'Call submit_result with arguments {"summary":"structured-ok","ok":true}. Do not wrap them in payload.',
      { label: "structured", schema: StructuredSmoke },
    );
    const labels = Array.from({ length: fanout }, (_, index) => `FANOUT-${index}`);
    const findings = await parallel(
      labels.map((label) => () =>
        agent(`Do not inspect files. Reply exactly: ${label}`, { label }),
      ),
    );
    return { finding, findings };
  },
});

console.log(
  JSON.stringify(
    {
      result,
    },
    null,
    2,
  ),
);
