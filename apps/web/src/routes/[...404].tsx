import { Title } from "@solidjs/meta";
import { A } from "@solidjs/router";

export default function NotFound() {
  return (
    <>
      <Title>404 - prismoid</Title>
      <div class="error-page">
        <h1>404</h1>
        <p>Page not found.</p>
        <A href="/" class="btn btn-outline">
          Back to home
        </A>
      </div>
    </>
  );
}
