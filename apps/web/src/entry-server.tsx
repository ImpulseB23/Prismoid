import { createHandler, StartServer } from "@solidjs/start/server";

export default createHandler(() => (
  <StartServer
    document={({ assets, children, scripts }) => (
      <html lang="en" style="color-scheme: dark">
        <head>
          <meta charset="utf-8" />
          <meta name="viewport" content="width=device-width, initial-scale=1" />
          <link
            rel="icon"
            type="image/svg+xml"
            href="/icons/icon-dark.svg"
            media="(prefers-color-scheme: light)"
          />
          <link
            rel="icon"
            type="image/svg+xml"
            href="/icons/icon-light.svg"
            media="(prefers-color-scheme: dark)"
          />
          <link
            rel="icon"
            type="image/png"
            sizes="32x32"
            href="/icons/icon-32.png"
          />
          <link
            rel="apple-touch-icon"
            sizes="180x180"
            href="/icons/apple-touch-icon.png"
          />
          <link rel="manifest" href="/site.webmanifest" />
          <meta name="theme-color" content="#6B3FA0" />
          <link
            rel="preload"
            href="/fonts/jetbrains-mono-latin.woff2"
            as="font"
            type="font/woff2"
            crossorigin=""
          />
          <link
            rel="preload"
            href="/fonts/outfit-latin.woff2"
            as="font"
            type="font/woff2"
            crossorigin=""
          />
          <style
            innerHTML={
              '@font-face{font-family:"JetBrains Mono";font-style:normal;font-weight:400 800;font-display:swap;src:url("/fonts/jetbrains-mono-latin.woff2") format("woff2")}@font-face{font-family:"Outfit";font-style:normal;font-weight:300 600;font-display:swap;src:url("/fonts/outfit-latin.woff2") format("woff2")}html{color-scheme:dark}body{background:#0e0e10;color:#efeff1;font-family:"Outfit",sans-serif;margin:0}'
            }
          />
          {assets}
        </head>
        <body>
          <div id="app">{children}</div>
          {scripts}
        </body>
      </html>
    )}
  />
));
