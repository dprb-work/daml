// Verification-only extension: drive the installed OMP binary's built-in LSP tool.
// No model prompt, editor integration, or source-checkout import is involved.
export default function (pi) {
  pi.on('session_start', async (_event, ctx) => {
    const report = {cwd: ctx.cwd, steps: [], passed: false};
    function check(condition, message) {
      if (!condition) throw new Error(message);
    }
    try {
      const settings = pi.pi.Settings.isolated({'lsp.shared': false});
      const lsp = new pi.pi.LspTool({cwd: ctx.cwd, hasUI: false, settings, enableLsp: true});
      async function call(params) {
        const result = await lsp.execute(`daml-distribution-${report.steps.length}`, params);
        report.steps.push({params, result});
        check(result.details?.success === true, `OMP ${params.action} failed`);
        return result.content.filter(item => item.type === 'text').map(item => item.text).join('\n');
      }
      const capabilities = await call({action: 'capabilities', file: 'trial.daml'});
      const caps = JSON.parse(capabilities.slice(capabilities.indexOf('{')));
      check(caps.positionEncoding === 'utf-16', 'expected UTF-16 negotiation');
      check(caps.documentFormattingProvider === true, 'formatting not advertised');
      const invalid = await call({action: 'diagnostics', file: 'trial.daml'});
      check(invalid.includes('1:7 [error] [daml]'), 'Unicode syntax diagnostic has the wrong position');
      const path = `${ctx.cwd}/trial.daml`;
      await Bun.write(path, '{message:"😀é"}');
      check(await call({action: 'diagnostics', file: 'trial.daml'}) === 'OK', 'replacement did not clear diagnostics');
      const uri = Bun.pathToFileURL(path).href;
      const request = {action: 'request', file: 'trial.daml', query: 'textDocument/formatting', payload: JSON.stringify({textDocument: {uri}, options: {tabSize: 2, insertSpaces: true}})};
      function edits(text) {
        return JSON.parse(text.slice(text.indexOf('\n') + 1));
      }
      const proposed = edits(await call(request));
      check(proposed.length === 1, 'expected one whole-document formatting edit');
      check(proposed[0].range.start.line === 0 && proposed[0].range.start.character === 0 && proposed[0].range.end.line === 0 && proposed[0].range.end.character === 15, 'wrong UTF-16 replacement range');
      check(proposed[0].newText === '{ message: "😀é" }\n', 'unexpected canonical output');
      await Bun.write(path, proposed[0].newText);
      check(await call({action: 'diagnostics', file: 'trial.daml'}) === 'OK', 'formatted replacement is invalid');
      check(edits(await call(request)).length === 0, 'formatting is not idempotent');
      const formatter = Bun.spawn([process.env.DAML_PROBE_FORMATTER, '--check', path], {stdout: 'pipe', stderr: 'pipe'});
      check(await formatter.exited === 0, 'installed daml-fmt rejected the OMP-applied result');
      report.finalSource = await Bun.file(path).text();
      report.passed = true;
    } catch (error) {
      report.error = String(error);
    } finally {
      await Bun.write(process.env.DAML_PROBE_REPORT, JSON.stringify(report, null, 2));
      ctx.shutdown();
    }
  });
}
