import { Router, A } from "@solidjs/router";
import { FileRoutes } from "@solidjs/start/router";
import { ErrorBoundary, Suspense } from "solid-js";
import { MetaProvider, Title } from "@solidjs/meta";

import "./global.css";
import "./layout.css";

function ErrorFallback(err: Error) {
  return (
    <>
      <Title>error - prismoid</Title>
      <div class="error-page">
        <h1>500</h1>
        <p>Something went wrong.</p>
        <p class="error-detail">{err.message}</p>
        <A href="/" class="btn btn-outline">
          Back to home
        </A>
      </div>
    </>
  );
}

export default function App() {
  return (
    <Router
      root={(props) => (
        <MetaProvider>
          <div class="site-wrapper">
            <header class="site-header">
              <div class="container">
                <nav>
                  <a href="/" class="logo">
                    <picture>
                      <source
                        srcset="/icons/nav-icon-light.svg"
                        media="(prefers-color-scheme: dark)"
                      />
                      <img
                        src="/icons/nav-icon-dark.svg"
                        alt=""
                        width="24"
                        height="20"
                        style="vertical-align: middle; margin-right: 8px;"
                      />
                    </picture>
                    prismoid<span class="logo-dot">.</span>
                  </a>
                  <div class="nav-links">
                    <a href="/readme" class="nav-link">
                      Readme
                    </a>
                    <a href="/contributing" class="nav-link">
                      Contribute
                    </a>
                  </div>
                </nav>
              </div>
            </header>
            <main>
              <div class="container">
                <ErrorBoundary fallback={(err) => <ErrorFallback {...err} />}>
                  <Suspense>{props.children}</Suspense>
                </ErrorBoundary>
              </div>
            </main>
            <div class="site-footer">
              <div class="container">
                <footer>
                  <span>&copy; {new Date().getFullYear()} prismoid</span>
                  <span>GPL-3.0</span>
                </footer>
              </div>
            </div>
          </div>
        </MetaProvider>
      )}
    >
      <FileRoutes />
    </Router>
  );
}
