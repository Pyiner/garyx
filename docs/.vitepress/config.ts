import { defineConfig } from "vitepress";

// Project pages live under https://pyiner.github.io/garyx/. If we ever
// move to a custom domain (e.g. garyx.dev), set `base` to "/" and add a
// docs/public/CNAME file.
export default defineConfig({
  base: "/garyx/",
  lang: "en-US",
  title: "Garyx",
  description:
    "Local-first AI gateway: connects CLI, HTTP/WS API, MCP tools, and channel bots to provider agents (Claude / Codex / Gemini) with shared thread history.",
  cleanUrls: true,
  lastUpdated: true,

  head: [
    ["link", { rel: "icon", href: "/garyx/favicon.svg", type: "image/svg+xml" }],
    [
      "meta",
      {
        name: "og:description",
        content:
          "Local-first AI gateway connecting Telegram, Feishu, WeChat, and a desktop app to Claude / Codex / Gemini agents.",
      },
    ],
  ],

  themeConfig: {
    siteTitle: "Garyx",
    logo: "/logo.svg",

    nav: [
      { text: "Docs", link: "/", activeMatch: "^/(?!architecture)" },
      {
        text: "v0.1.10",
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
