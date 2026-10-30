import init, {
  Bundle,
  Schemas,
  formatDefinition,
  parse,
  write,
  type NodeOrigin,
  type SemanticNode,
} from '../dist/daml_browser.js';

export async function consumePublicSdkTypes(
  wasmUrl: URL,
  source: string,
  sources: Record<string, string>,
): Promise<string> {
  await init(wasmUrl);
  const document = parse(source);
  const originalSource: string = document.source;
  const parsedValue: SemanticNode = document.value();
  const formattedDocument: string = document.format();
  const formattedDefinition: string = formatDefinition(sources['Config.daml'] ?? '', 80);

  const bundle = new Bundle('.', sources);
  const schemas = bundle.compile(document, 'app.Config');
  const resolution = schemas.resolve(document, 'app.Config');
  const resolvedSource: string = resolution.source;
  const resolvedValue: SemanticNode = resolution.value();
  const origins: Record<string, NodeOrigin> = resolution.origins();
  const rootOrigin: NodeOrigin | undefined = origins[''];
  if (rootOrigin?.kind === 'input') {
    const originalOffset: number = rootOrigin.span.start;
    void originalOffset;
  }

  const exactInteger: SemanticNode = { kind: 'number', value: '9007199254740993' };
  // @ts-expect-error Number transport is exact decimal text, never a JavaScript number.
  const unsafeInteger: SemanticNode = { kind: 'number', value: 9007199254740993 };
  const canonical: string = schemas.write(resolvedValue);
  const plainParsed: string = write(parsedValue, 80);
  const plainInteger: string = write(exactInteger);
  void unsafeInteger;

  resolution.free();
  schemas.free();
  bundle.free();
  document.free();
  return [originalSource, resolvedSource, formattedDocument, formattedDefinition,
    canonical, plainParsed, plainInteger].join('\n');
}

export function consumeGeneratedSchemaTypes(
  moduleSources: readonly string[],
  value: SemanticNode,
): void {
  const schemas = Schemas.fromJsonModules(moduleSources);
  try {
    schemas.validateSemantic(value, 'app.GeneratedJob');
  } finally {
    schemas.free();
  }
}
