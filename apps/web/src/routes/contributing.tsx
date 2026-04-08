import { Title } from "@solidjs/meta";
import { query, createAsync } from "@solidjs/router";
import { Suspense } from "solid-js";
import { parseMarkdown } from "~/lib/markdown";
import "./contributing.css";

const getContributing = query(async () => {
  "use server";
  const res = await fetch(
    "https://raw.githubusercontent.com/ImpulseB23/Prismoid/main/CONTRIBUTING.md",
  );
  if (!res.ok) throw new Error(`Failed to fetch: ${res.status}`);
  return parseMarkdown(await res.text());
}, "contributing");

export const route = { preload: () => getContributing() };

export default function Contributing() {
  const html = createAsync(() => getContributing());

  return (
    <>
      <Title>contributing - prismoid</Title>
      <article class="prose">
        <Suspense>
          <div innerHTML={html()} />
        </Suspense>
      </article>
    </>
  );
}
