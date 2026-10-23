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
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
