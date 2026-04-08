import { marked } from "marked";

const REPO_URL = "https://github.com/ImpulseB23/Prismoid";
const BLOB_BASE = `${REPO_URL}/blob/main`;
const RAW_BASE = "https://raw.githubusercontent.com/ImpulseB23/Prismoid/main";

function isRelativeRef(href: string): boolean {
  return (
    !href.startsWith("http") && !href.startsWith("#") && !href.startsWith("/")
  );
}

function rewriteRelativeLink(href: string): string {
  if (
    href === "CONTRIBUTING.md" ||
    href === "./CONTRIBUTING.md" ||
    href === "contributing.md" ||
    href === "./contributing.md"
  ) {
    return "/contributing";
  }

  if (href === "docs" || href === "docs/" || href === "./docs/") {
    return `${REPO_URL}/tree/main/docs`;
  }

  if (href.startsWith("docs/")) {
    return `${REPO_URL}/tree/main/${href}`;
  }

  if (href.startsWith("./docs/")) {
    return `${REPO_URL}/tree/main/${href.slice(2)}`;
  }

  return `${BLOB_BASE}/${href}`;
}

function rewriteHtmlRefs(html: string): string {
  const withoutCenter = html.replace(/\salign=("|')center\1/gi, "");

  const withHref = withoutCenter.replace(
    /(href\s*=\s*["'])([^"']+)(["'])/gi,
    (_m, prefix: string, href: string, suffix: string) => {
      if (!isRelativeRef(href)) return `${prefix}${href}${suffix}`;
      return `${prefix}${rewriteRelativeLink(href)}${suffix}`;
    },
  );

  return withHref.replace(
    /(src\s*=\s*["'])([^"']+)(["'])/gi,
    (_m, prefix: string, src: string, suffix: string) => {
      if (!isRelativeRef(src)) return `${prefix}${src}${suffix}`;
      return `${prefix}${RAW_BASE}/${src}${suffix}`;
    },
  );
}

function stripReadmeLogo(html: string): string {
  return html.replace(/<picture[\s\S]*?<\/picture>/gi, "");
}

marked.use({
  hooks: {
    postprocess(html) {
      return rewriteHtmlRefs(html);
    },
  },
  walkTokens(token) {
    if (token.type === "link") {
      const href = token.href;
      if (href && isRelativeRef(href)) {
        token.href = rewriteRelativeLink(href);
      }
    }

    if (token.type === "image") {
      const href = token.href;
      if (href && isRelativeRef(href)) {
        token.href = `${RAW_BASE}/${href}`;
      }
    }
  },
});

export function parseMarkdown(md: string): string {
  return marked.parse(md) as string;
}

export function parseReadmeMarkdown(md: string): string {
  return stripReadmeLogo(parseMarkdown(md));
}
