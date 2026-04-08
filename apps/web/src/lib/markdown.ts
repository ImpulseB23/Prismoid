import { marked } from "marked";

const REPO_URL = "https://github.com/ImpulseB23/Prismoid";
const BLOB_BASE = `${REPO_URL}/blob/main`;

marked.use({
  walkTokens(token) {
    if (token.type === "link") {
      const href = token.href;
      if (
        href &&
        !href.startsWith("http") &&
        !href.startsWith("#") &&
        !href.startsWith("/")
      ) {
        token.href = `${BLOB_BASE}/${href}`;
      }
    }
  },
});

export function parseMarkdown(md: string): string {
  return marked.parse(md) as string;
}
