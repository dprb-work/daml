# Browser SDK verifier

This isolated package drives the built static SDK in a real Chromium browser and type-checks a small consumer against the generated declarations. It requires Node.js 20 or newer, uses pinned Playwright 1.64.1 and TypeScript 6.0.3, and does not import the SDK's Cargo/package manifest or use a mock parser/compiler. The verifier serves only `dist/daml_browser.js` and `dist/daml_browser_bg.wasm` from the adjacent binding package; it allows the static preload, then blocks and reports every subsequent browser request.

From the repository root, first produce the canonical source through the explicit native probe using the shared fixture, then install the pinned local development dependency and run the browser scenario:

```sh
cargo run --manifest-path bindings/browser/Cargo.toml --example native-parity -- bindings/browser/verify/fixtures/native-parity.json > /tmp/daml-native-parity.json
cd bindings/browser/verify
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci
npm run typecheck
CHROMIUM_PATH=/path/to/chromium npm run verify -- --dist=../dist --native-result=/tmp/daml-native-parity.json
```

The browser compares tagged and tagless `Schemas.write` output against the actual Rust probe (using expected-root resolution), checks tagless defaults and invalid-value rejection, and verifies that a default `self.Base` tag rebases to the visible alias `app.Base` and survives export/re-admission with the same value and the `app.Base` assertion rather than a same-shaped root `self.Base`. Explicit parsed tags intentionally do not carry native binding metadata. It also checks exact lossless wire values and native default provenance. `CHROMIUM_PATH` must identify an already-installed Chromium executable; `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1` avoids fetching a browser during dependency installation.

For a visible inspection session, run `CHROMIUM_PATH=/path/to/chromium npm run verify -- --dist=../dist --native-result=/tmp/daml-native-parity.json --headed --hold --cdp-port=9222`. This leaves Chromium open until Ctrl-C and enables a loopback CDP port for an authorized browser client. It does not publish or expose the local static server beyond loopback.
