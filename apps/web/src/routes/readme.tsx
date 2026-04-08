import { Title } from "@solidjs/meta";
import { query, createAsync } from "@solidjs/router";
import { Suspense } from "solid-js";
import { parseMarkdown } from "~/lib/markdown";
import "./contributing.css";

const getReadme = query(async () => {
  "use server";
  const res = await fetch(
    "https://raw.githubusercontent.com/ImpulseB23/Prismoid/main/README.md",
  );
  if (!res.ok) throw new Error(`Failed to fetch: ${res.status}`);
  return parseMarkdown(await res.text());
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
