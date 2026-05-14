import { defineConfig } from "vitepress";

// Project pages live under https://pyiner.github.io/garyx/. If we ever
// move to a custom domain (e.g. garyx.dev), set `base` to "/" and add a
// docs/public/CNAME file.
export default defineConfig({
  base: "/garyx/",
  lang: "en-US",
  title: "Garyx",
  description:
    "Local-first AI agent gateway connecting Telegram, Feishu / Lark, WeChat, CLI, HTTP/WS API, MCP, automations, and the macOS app to Claude Code, Codex, and Gemini.",
  cleanUrls: true,
  lastUpdated: true,

  head: [
    ["link", { rel: "icon", href: "/garyx/favicon.svg", type: "image/svg+xml" }],
    [
      "meta",
      {
        name: "og:description",
        content:
          "Local-first AI agent gateway connecting channel bots, CLI, API, MCP, automations, and the macOS app to provider-backed agents.",
      },
    ],
  ],

  themeConfig: {
    siteTitle: "Garyx",
    logo: "/logo.svg",

    nav: [
      { text: "Docs", link: "/", activeMatch: "^/(?!architecture)" },
      {
        text: "v0.1.23",
        items: [
          { text: "Releases", link: "https://github.com/Pyiner/garyx/releases" },
          { text: "Changelog", link: "https://github.com/Pyiner/garyx/commits/main" },
        ],
      },
    ],

    sidebar: {
      "/": [
        { text: "Introduction", link: "/" },
        {
          text: "Get started",
          items: [
            { text: "Installation", link: "/installation" },
            { text: "Your first bot", link: "/first-bot" },
          ],
        },
        {
          text: "Concepts",
          items: [
            { text: "Threads & workspaces", link: "/concepts/threads-and-workspaces" },
            { text: "Channels", link: "/concepts/channels" },
            { text: "Providers", link: "/concepts/providers" },
            { text: "MCP integration", link: "/concepts/mcp" },
          ],
        },
        {
          text: "Reference",
          items: [
            { text: "Configuration", link: "/configuration" },
            { text: "CLI commands", link: "/reference/cli" },
            { text: "Service manager", link: "/reference/service-manager" },
            { text: "Security", link: "/security" },
            {
              text: "Architecture: command list",
              link: "/architecture/command-list-design",
            },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: "github", link: "https://github.com/Pyiner/garyx" },
    ],

    editLink: {
      pattern: "https://github.com/Pyiner/garyx/edit/main/docs/:path",
      text: "Edit this page on GitHub",
    },

    footer: {
      message: "Released under the MIT License.",
      copyright: "© Pyiner",
    },

    search: { provider: "local" },
  },
});
