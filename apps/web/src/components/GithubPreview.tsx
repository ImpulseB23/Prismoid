import { createResource, createSignal, Show } from "solid-js";
import "./GithubPreview.css";

const REPO = "ImpulseB23/Prismoid";
const REPO_URL = `https://github.com/${REPO}`;
const CACHE_TTL = 5 * 60 * 1000;

interface RepoInfo {
  stars: number;
  openIssues: number;
  openPRs: number;
  lastCommitMsg: string;
  lastCommitSha: string;
  lastCommitAuthor: string;
  lastCommitTime: string;
}

let cached: { data: RepoInfo; ts: number } | null = null;

async function fetchRepoInfo(): Promise<RepoInfo | null> {
  if (cached && Date.now() - cached.ts < CACHE_TTL) return cached.data;

  try {
    const [repoRes, issuesRes, prsRes, commitsRes] = await Promise.all([
      fetch(`https://api.github.com/repos/${REPO}`),
      fetch(
        `https://api.github.com/repos/${REPO}/issues?state=open&per_page=100`,
      ),
      fetch(
        `https://api.github.com/repos/${REPO}/pulls?state=open&per_page=100`,
      ),
      fetch(`https://api.github.com/repos/${REPO}/commits?per_page=1`),
    ]);

    if (!repoRes.ok || !commitsRes.ok) return cached?.data ?? null;

    const repo = await repoRes.json();
    const issues = await issuesRes.json();
    const prs = await prsRes.json();
    const commits = await commitsRes.json();
    const latest = commits[0];

    if (!latest?.commit) return cached?.data ?? null;

    const data: RepoInfo = {
      stars: repo.stargazers_count ?? 0,
      openIssues: Array.isArray(issues)
        ? issues.filter((i: any) => !i.pull_request).length
        : 0,
      openPRs: Array.isArray(prs) ? prs.length : 0,
      lastCommitMsg: latest.commit.message.split("\n")[0],
      lastCommitSha: latest.sha,
      lastCommitAuthor: latest.commit.author.name,
      lastCommitTime: timeAgo(new Date(latest.commit.author.date)),
    };

    cached = { data, ts: Date.now() };
    return data;
  } catch {
    return cached?.data ?? null;
  }
}

function timeAgo(date: Date): string {
  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export default function GithubPreview(props: {
  children: any;
  href: string;
  class?: string;
  target?: string;
}) {
  const [show, setShow] = createSignal(false);
  const [info] = createResource(fetchRepoInfo);
  let timeout: number;

  const keepOpen = () => {
    clearTimeout(timeout);
  };

  const onButtonEnter = () => {
    clearTimeout(timeout);
    setShow(true);
  };

  const onLeave = () => {
    timeout = window.setTimeout(() => setShow(false), 300);
  };

  return (
    <span class="gh-preview-wrapper" onMouseLeave={onLeave}>
      <a
        href={props.href}
        class={props.class}
        target={props.target}
        rel={props.target === "_blank" ? "noopener" : undefined}
        onMouseEnter={onButtonEnter}
      >
        {props.children}
      </a>
      <Show when={show()}>
        <div class="gh-preview-bridge" onMouseEnter={keepOpen} />
      </Show>
      <Show when={show() && info()}>
        <div class="gh-preview-card" onMouseEnter={keepOpen}>
          <div class="gh-preview-stats">
            <a href={`${REPO_URL}/stargazers`} target="_blank" rel="noopener">
              &#9733; {info()!.stars}
            </a>
            <a href={`${REPO_URL}/pulls`} target="_blank" rel="noopener">
              {info()!.openPRs === 0
                ? "no PRs"
                : `${info()!.openPRs} ${info()!.openPRs === 1 ? "PR" : "PRs"}`}
            </a>
            <a href={`${REPO_URL}/issues`} target="_blank" rel="noopener">
              {info()!.openIssues === 0
                ? "no issues"
                : `${info()!.openIssues} ${info()!.openIssues === 1 ? "issue" : "issues"}`}
            </a>
          </div>
          <div class="gh-preview-commit">
            <span class="gh-preview-label">latest commit</span>
            <a
              href={`${REPO_URL}/commit/${info()!.lastCommitSha}`}
              target="_blank"
              rel="noopener"
              class="gh-preview-msg"
            >
              {info()!.lastCommitMsg}
            </a>
            <span class="gh-preview-meta">
              {info()!.lastCommitAuthor} &middot; {info()!.lastCommitTime}
            </span>
          </div>
        </div>
      </Show>
    </span>
  );
}
