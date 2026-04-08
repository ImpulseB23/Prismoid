import { Title } from "@solidjs/meta";
import { query, createAsync } from "@solidjs/router";
import { Suspense } from "solid-js";
import { parseReadmeMarkdown } from "~/lib/markdown";
import "./contributing.css";

const getReadme = query(async () => {
  "use server";

  if (import.meta.env.DEV) {
    const { readFile } = await import("node:fs/promises");
    const { join } = await import("node:path");
    const md = await readFile(
      join(process.cwd(), "..", "..", "README.md"),
      "utf8",
    );
    return parseReadmeMarkdown(md);
  }

  const res = await fetch(
    "https://raw.githubusercontent.com/ImpulseB23/Prismoid/main/README.md",
  );
  if (!res.ok) throw new Error(`Failed to fetch: ${res.status}`);
  return parseReadmeMarkdown(await res.text());
}, "readme");

export const route = { preload: () => getReadme() };

export default function Readme() {
  const html = createAsync(() => getReadme());

  return (
    <>
      <Title>readme - prismoid</Title>
      <article class="prose">
        <Suspense>
          <div innerHTML={html()} />
        </Suspense>
      </article>
    </>
  );
}
