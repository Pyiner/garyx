import { defineConfig } from "vitepress";

// Project pages live under https://pyiner.github.io/garyx/. If we ever
// move to garyx.github.io or a custom domain, set `base` to "/" and
// update the editLink + socialLinks.
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
      {
        text: "Docs",
        link: "/getting-started",
        activeMatch: "^/(getting-started|configuration)",
      },
      {
        text: "Architecture",
        link: "/architecture/command-list-design",
        activeMatch: "^/architecture/",
      },
      {
        text: "v0.1",
        items: [
          { text: "Releases", link: "https://github.com/Pyiner/garyx/releases" },
          { text: "Changelog", link: "https://github.com/Pyiner/garyx/commits/main" },
        ],
      },
    ],

    sidebar: {
      "/": [
        {
          text: "Introduction",
          items: [
            { text: "What is Garyx", link: "/" },
            { text: "Getting Started", link: "/getting-started" },
          ],
        },
        {
          text: "Reference",
          items: [{ text: "Configuration", link: "/configuration" }],
        },
        {
          text: "Architecture",
          items: [
            {
              text: "Command List Design",
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

    search: {
      provider: "local",
    },
  },
});
