const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vscodeOniguruma = require('vscode-oniguruma');
const vscodeTextmate = require('vscode-textmate');

const repo = path.resolve(__dirname, '../../..');
const grammarPath = process.env.DAML_GRAMMAR_PATH || path.join(repo, 'integrations/vscode/syntaxes/daml.tmLanguage.json');
const grammar = JSON.parse(fs.readFileSync(grammarPath, 'utf8'));
const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm'));

async function main() {
  await vscodeOniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const registry = new vscodeTextmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: sources => new vscodeOniguruma.OnigScanner(sources),
      createOnigString: value => new vscodeOniguruma.OnigString(value),
    }),
    loadGrammar: async scopeName => scopeName === 'source.daml' ? grammar : null,
  });
  const tokenizer = await registry.loadGrammar('source.daml');
  const scopesAt = (line, offset, ruleStack) => {
    const result = tokenizer.tokenizeLine(line, ruleStack);
    const token = result.tokens.find(candidate => candidate.startIndex <= offset && offset < candidate.endIndex);
    assert.ok(token, `No token at offset ${offset} in ${JSON.stringify(line)}`);
    return token.scopes;
  };
  const has = (scopes, expected) => assert.ok(scopes.includes(expected), `Expected ${expected} in ${scopes.join(' ')}`);
  const lacks = (scopes, rejected) => assert.ok(!scopes.includes(rejected), `Unexpected ${rejected} in ${scopes.join(' ')}`);

  let stack = vscodeTextmate.INITIAL;
  const escapedBackslash = '"ends with slash \\\\" # trailing comment';
  const afterQuote = escapedBackslash.indexOf('#');
  const afterQuoteScopes = scopesAt(escapedBackslash, afterQuote, stack);
  has(afterQuoteScopes, 'comment.line.number-sign.daml');
  lacks(afterQuoteScopes, 'string.quoted.double.daml');
  has(scopesAt(escapedBackslash, escapedBackslash.indexOf('\\\\'), stack), 'constant.character.escape.daml');

  const escapedQuote = '"quoted \\\" text" # comment';
  const escapedQuoteScopes = scopesAt(escapedQuote, escapedQuote.indexOf('\\') + 1, stack);
  has(escapedQuoteScopes, 'constant.character.escape.daml');
  has(scopesAt(escapedQuote, escapedQuote.indexOf('text'), stack), 'string.quoted.double.daml');
  has(scopesAt(escapedQuote, escapedQuote.indexOf('#'), stack), 'comment.line.number-sign.daml');

  stack = tokenizer.tokenizeLine('"""', stack).ruleStack;
  const tripleBody = '# remains part of triple string';
  has(scopesAt(tripleBody, 0, stack), 'string.quoted.triple.double.daml');
  stack = tokenizer.tokenizeLine(tripleBody, stack).ruleStack;
  stack = tokenizer.tokenizeLine('"""', stack).ruleStack;
  const afterTriple = 'value # comment';
  has(scopesAt(afterTriple, afterTriple.indexOf('#'), stack), 'comment.line.number-sign.daml');

  const rawRegex = '!Regex "a\\\\b#still-regex" # comment';
  has(scopesAt(rawRegex, rawRegex.indexOf('\\\\') + 1, stack), 'string.regexp.daml');
  lacks(scopesAt(rawRegex, rawRegex.indexOf('\\\\') + 1, stack), 'constant.character.escape.daml');
  has(scopesAt(rawRegex, rawRegex.indexOf('#'), stack), 'string.regexp.daml');
  has(scopesAt(rawRegex, rawRegex.lastIndexOf('#'), stack), 'comment.line.number-sign.daml');

  // Drive the real rule stack through native raw multiline payloads. A # is
  // regex data, not a DAML comment, even when the regex engine uses (?x).
  for (const delimiter of ['"""', "'''"]) {
    for (const gap of ['', ' ', '\t']) {
      let state = vscodeTextmate.INITIAL;
      const opener = `pattern: !Regex${gap}${delimiter}`;
      has(scopesAt(opener, opener.indexOf('Regex'), state), 'entity.name.type.tag.daml');
      state = tokenizer.tokenizeLine(opener, state).ruleStack;
      const body = String.raw`  (?x)\d+\\x # regex comment`;
      for (const offset of [body.indexOf('\\'), body.indexOf('#')]) {
        has(scopesAt(body, offset, state), 'string.regexp.daml');
        lacks(scopesAt(body, offset, state), 'constant.character.escape.daml');
        lacks(scopesAt(body, offset, state), 'comment.line.number-sign.daml');
      }
      state = tokenizer.tokenizeLine(body, state).ruleStack;
      for (const count of [1, 3]) {
        const escapedDelimiter = '  a ' + '\\'.repeat(count) + delimiter + ' b';
        has(scopesAt(escapedDelimiter, escapedDelimiter.indexOf('b'), state), 'string.regexp.daml');
        state = tokenizer.tokenizeLine(escapedDelimiter, state).ruleStack;
      }
      const closing = `  ${delimiter} # outside`;
      has(scopesAt(closing, closing.indexOf('#'), state), 'comment.line.number-sign.daml');
      lacks(scopesAt(closing, closing.indexOf('#'), state), 'string.regexp.daml');
      state = tokenizer.tokenizeLine(closing, state).ruleStack;
      const field = 'next: !String # following field';
      lacks(scopesAt(field, 0, state), 'string.regexp.daml');
      has(scopesAt(field, field.indexOf('#'), state), 'comment.line.number-sign.daml');
    }
  }

  // Native raw single quotes terminate at a quote regardless of backslashes.
  const rawQuote = String.raw`!Regex "a\" # outside`;
  has(scopesAt(rawQuote, rawQuote.indexOf('#'), vscodeTextmate.INITIAL), 'comment.line.number-sign.daml');
  lacks(scopesAt(rawQuote, rawQuote.indexOf('#'), vscodeTextmate.INITIAL), 'string.regexp.daml');

  // Constructor dispatch is horizontal only; the next line is an ordinary
  // triple string and retains ordinary escape highlighting.
  stack = tokenizer.tokenizeLine('!Regex', vscodeTextmate.INITIAL).ruleStack;
  stack = tokenizer.tokenizeLine('"""', stack).ruleStack;
  const ordinaryBody = String.raw`  a \\ b`;
  has(scopesAt(ordinaryBody, ordinaryBody.indexOf('\\'), stack), 'constant.character.escape.daml');
  lacks(scopesAt(ordinaryBody, ordinaryBody.indexOf('\\'), stack), 'string.regexp.daml');
  stack = tokenizer.tokenizeLine(ordinaryBody, stack).ruleStack;
  stack = tokenizer.tokenizeLine('"""', stack).ruleStack;
  const temporal = '!Date """';
  stack = tokenizer.tokenizeLine(temporal, stack).ruleStack;
  has(scopesAt('  2024-02-29', 2, stack), 'string.quoted.triple.double.daml');
  lacks(scopesAt('  2024-02-29', 2, stack), 'string.regexp.daml');
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
