import fs from 'fs'
import path from 'path'
import Parser from 'tree-sitter'
import JavaScript from 'tree-sitter-javascript'
import TypeScript from 'tree-sitter-typescript'
import Database from 'better-sqlite3'

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const SCOPE_DIR = path.join(ROOT, '.scope')
const DB_PATH = path.join(SCOPE_DIR, 'poc.db')
const JS_EXTENSIONS = new Set(['.js', '.jsx', '.mjs', '.cjs'])
const TS_EXTENSIONS = new Set(['.ts', '.tsx'])
const SOURCE_EXTENSIONS = new Set([...JS_EXTENSIONS, ...TS_EXTENSIONS])
const IGNORED_DIRS = new Set(['.git', '.scope', 'node_modules', 'target'])

fs.mkdirSync(SCOPE_DIR, { recursive: true })

const jsParser = new Parser()
jsParser.setLanguage(JavaScript)
const tsParser = new Parser()
tsParser.setLanguage(TypeScript.typescript)

const db = new Database(DB_PATH)
db.pragma('journal_mode = WAL')
db.exec(`
  CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    language TEXT NOT NULL
  );

  CREATE TABLE IF NOT EXISTS imports (
    from_file TEXT NOT NULL,
    to_file TEXT NOT NULL,
    raw_path TEXT NOT NULL,
    UNIQUE(from_file, to_file, raw_path)
  );

  CREATE TABLE IF NOT EXISTS exports (
    file TEXT NOT NULL,
    symbol TEXT NOT NULL,
    UNIQUE(file, symbol)
  );

  CREATE TABLE IF NOT EXISTS functions (
    file TEXT NOT NULL,
    name TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    UNIQUE(file, name, start_line)
  );

  CREATE TABLE IF NOT EXISTS calls (
    from_file TEXT NOT NULL,
    from_fn TEXT NOT NULL,
    to_fn TEXT NOT NULL,
    line INTEGER NOT NULL,
    UNIQUE(from_file, from_fn, to_fn, line)
  );
`)

const clearTables = db.transaction(() => {
  db.exec('DELETE FROM files; DELETE FROM imports; DELETE FROM exports; DELETE FROM functions; DELETE FROM calls;')
})

const insertFile = db.prepare('INSERT INTO files (path, language) VALUES (?, ?)')
const insertImport = db.prepare('INSERT INTO imports (from_file, to_file, raw_path) VALUES (?, ?, ?)')
const insertExport = db.prepare('INSERT INTO exports (file, symbol) VALUES (?, ?)')
const insertFunction = db.prepare('INSERT INTO functions (file, name, start_line) VALUES (?, ?, ?)')
const insertCall = db.prepare('INSERT INTO calls (from_file, from_fn, to_fn, line) VALUES (?, ?, ?, ?)')
const countRow = (table) => db.prepare(`SELECT COUNT(*) AS count FROM ${table}`).get().count

function parserForFile(filePath) {
  const ext = path.extname(filePath)
  if (TS_EXTENSIONS.has(ext)) return { parser: tsParser, language: 'typescript' }
  if (JS_EXTENSIONS.has(ext)) return { parser: jsParser, language: 'javascript' }
  return null
}

function isSourceFile(filePath) {
  return SOURCE_EXTENSIONS.has(path.extname(filePath))
}

function walkFiles(rootDir) {
  const out = []

  function visit(current) {
    const stat = fs.statSync(current)
    if (stat.isDirectory()) {
      if (IGNORED_DIRS.has(path.basename(current))) return
      for (const entry of fs.readdirSync(current)) {
        visit(path.join(current, entry))
      }
      return
    }

    if (stat.isFile() && isSourceFile(current)) {
      out.push(current)
    }
  }

  visit(rootDir)
  return out.sort()
}

function relativeToRoot(filePath) {
  return path.relative(ROOT, filePath).replaceAll(path.sep, '/')
}

function resolveImport(fromFile, importPath) {
  if (!importPath.startsWith('.')) return null

  const fromDir = path.dirname(fromFile)
  const base = path.resolve(fromDir, importPath)
  const candidates = [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    `${base}.js`,
    `${base}.jsx`,
    path.join(base, 'index.ts'),
    path.join(base, 'index.tsx'),
    path.join(base, 'index.js'),
    path.join(base, 'index.jsx'),
  ]

  for (const candidate of candidates) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return relativeToRoot(candidate)
    }
  }

  return path.relative(ROOT, base).replaceAll(path.sep, '/')
}

function textFor(node, source) {
  return source.slice(node.startIndex, node.endIndex)
}

function collectIdentifiers(node, source) {
  const out = []

  function visit(current) {
    if (current.type === 'identifier' || current.type === 'property_identifier') {
      out.push(textFor(current, source))
    }
    for (const child of current.namedChildren ?? []) visit(child)
  }

  visit(node)
  return out
}

function extractImports(rootNode, source, absolutePath) {
  const imports = []

  for (const child of rootNode.namedChildren) {
    if (child.type !== 'import_statement' && child.type !== 'export_statement') continue
    const sourceNode = child.namedChildren.find((node) => node.type === 'string')
    if (!sourceNode) continue
    const raw = textFor(sourceNode, source).slice(1, -1)
    const resolved = resolveImport(absolutePath, raw)
    if (resolved) {
      imports.push({ raw, resolved })
    }
  }

  return imports
}

function exportedNames(exportNode, source) {
  const declaration = exportNode.namedChildren.find((child) => child.type !== 'string')
  if (!declaration) return []

  if (declaration.type === 'export_clause' || declaration.type === 'named_exports') {
    return declaration.namedChildren
      .filter((child) => child.type === 'export_specifier')
      .map((specifier) => {
        const alias = specifier.childForFieldName?.('alias')
        const name = specifier.childForFieldName?.('name')
        const fallback = specifier.namedChildren.find((child) => child.type === 'identifier')
        return alias ? textFor(alias, source) : name ? textFor(name, source) : fallback ? textFor(fallback, source) : null
      })
      .filter(Boolean)
  }

  const name = functionName(declaration, source)
  if (name) return [name]

  if (declaration.type === 'class_declaration') {
    const className = declaration.childForFieldName?.('name')
    return className ? [textFor(className, source)] : []
  }

  return []
}

function extractExports(rootNode, source) {
  const exports = []

  for (const child of rootNode.namedChildren) {
    if (child.type !== 'export_statement') continue
    for (const identifier of exportedNames(child, source)) {
      exports.push(identifier)
    }
  }

  return [...new Set(exports)]
}

function functionName(node, source) {
  const nameNode = node.childForFieldName?.('name')
  if (nameNode) return textFor(nameNode, source)

  if (node.type === 'lexical_declaration' || node.type === 'variable_declaration') {
    const declarator = node.namedChildren.find((child) => child.type === 'variable_declarator')
    const name = declarator?.childForFieldName?.('name')
    return name ? textFor(name, source) : null
  }

  return null
}

function calleeName(node, source) {
  const fn = node.childForFieldName?.('function') ?? node.namedChildren?.[0]
  if (!fn) return null
  return textFor(fn, source)
}

function extractFunctionsAndCalls(rootNode, source) {
  const functions = []
  const calls = []

  function visit(node, currentFn = null) {
    let nextFn = currentFn
    const isFunction = [
      'function_declaration',
      'method_definition',
      'function',
      'arrow_function',
      'generator_function_declaration',
    ].includes(node.type)

    if (isFunction) {
      const name = functionName(node, source)
      if (name) {
        functions.push({ name, line: node.startPosition.row + 1 })
        nextFn = name
      }
    }

    if ((node.type === 'lexical_declaration' || node.type === 'variable_declaration')) {
      const declarator = node.namedChildren.find((child) => child.type === 'variable_declarator')
      const value = declarator?.childForFieldName?.('value')
      if (value && ['arrow_function', 'function'].includes(value.type)) {
        const name = functionName(node, source)
        if (name) {
          functions.push({ name, line: node.startPosition.row + 1 })
          nextFn = name
        }
      }
    }

    if (node.type === 'call_expression' && currentFn) {
      const target = calleeName(node, source)
      if (target) {
        calls.push({ from_fn: currentFn, to_fn: target, line: node.startPosition.row + 1 })
      }
    }

    for (const child of node.namedChildren ?? []) {
      visit(child, nextFn)
    }
  }

  visit(rootNode)
  return { functions, calls }
}

function extractFileInfo(absolutePath) {
  const parserInfo = parserForFile(absolutePath)
  if (!parserInfo) return null

  const source = fs.readFileSync(absolutePath, 'utf8')
  const tree = parserInfo.parser.parse(source)
  const rootNode = tree.rootNode

  const imports = extractImports(rootNode, source, absolutePath)
  const exports = extractExports(rootNode, source)
  const { functions, calls } = extractFunctionsAndCalls(rootNode, source)

  return {
    file: relativeToRoot(absolutePath),
    language: parserInfo.language,
    imports,
    exports,
    functions,
    calls,
  }
}

function indexDirectory(targetDir) {
  const absoluteTarget = path.resolve(process.cwd(), targetDir)
  const files = walkFiles(absoluteTarget)
  clearTables()

  for (const file of files) {
    const info = extractFileInfo(file)
    if (!info) continue

    insertFile.run(info.file, info.language)
    for (const entry of info.imports) insertImport.run(info.file, entry.resolved, entry.raw)
    for (const symbol of info.exports) insertExport.run(info.file, symbol)
    for (const fn of info.functions) insertFunction.run(info.file, fn.name, fn.line)
    for (const call of info.calls) insertCall.run(info.file, call.from_fn, call.to_fn, call.line)
  }

  return {
    command: 'index',
    status: 'ok',
    data: {
      target: relativeToRoot(absoluteTarget),
      database: relativeToRoot(DB_PATH),
      stats: {
        files: countRow('files'),
        imports: countRow('imports'),
        exports: countRow('exports'),
        functions: countRow('functions'),
        calls: countRow('calls'),
      },
    },
  }
}

function queryFile(target) {
  const relativeTarget = relativeToRoot(path.resolve(process.cwd(), target))
  return {
    command: 'file',
    status: 'ok',
    data: {
      file: relativeTarget,
      imports: db.prepare('SELECT to_file FROM imports WHERE from_file = ? ORDER BY to_file').all(relativeTarget).map((row) => row.to_file),
      imported_by: db.prepare('SELECT from_file FROM imports WHERE to_file = ? ORDER BY from_file').all(relativeTarget).map((row) => row.from_file),
      exports: db.prepare('SELECT symbol FROM exports WHERE file = ? ORDER BY symbol').all(relativeTarget).map((row) => row.symbol),
      functions: db.prepare('SELECT name, start_line FROM functions WHERE file = ? ORDER BY start_line, name').all(relativeTarget),
    },
  }
}

function queryFunction(name) {
  return {
    command: 'fn',
    status: 'ok',
    data: {
      function: name,
      defined_in: db.prepare('SELECT file, start_line FROM functions WHERE name = ? ORDER BY file, start_line').all(name),
      called_by: db.prepare('SELECT from_file, from_fn, line FROM calls WHERE to_fn = ? ORDER BY from_file, line').all(name),
      calls: db.prepare('SELECT to_fn, line FROM calls WHERE from_fn = ? ORDER BY line, to_fn').all(name),
    },
  }
}

function queryImpact(target) {
  const file = relativeToRoot(path.resolve(process.cwd(), target))
  const queue = [{ value: file, depth: 0 }]
  const visited = new Set([file])
  const direct = []
  const transitive = []

  while (queue.length > 0) {
    const current = queue.shift()
    const dependents = db.prepare('SELECT from_file FROM imports WHERE to_file = ? ORDER BY from_file').all(current.value)
    for (const dependent of dependents) {
      if (visited.has(dependent.from_file)) continue
      visited.add(dependent.from_file)
      if (current.depth === 0) {
        direct.push(dependent.from_file)
      } else {
        transitive.push(dependent.from_file)
      }
      queue.push({ value: dependent.from_file, depth: current.depth + 1 })
    }
  }

  const total = direct.length + transitive.length
  const risk = total >= 5 ? 'HIGH' : total >= 2 ? 'MEDIUM' : total >= 1 ? 'LOW' : 'NONE'

  return {
    command: 'impact',
    status: 'ok',
    data: {
      file,
      direct_dependents: direct,
      transitive_dependents: transitive,
      total_affected: total,
      risk,
    },
  }
}

const [, , command, target] = process.argv

const handlers = {
  index: () => indexDirectory(target || '.'),
  file: () => queryFile(target),
  fn: () => queryFunction(target),
  impact: () => queryImpact(target),
}

if (!handlers[command]) {
  console.log(JSON.stringify({
    status: 'error',
    error: 'Usage: node poc/index.js <index|file|fn|impact> <target>',
  }, null, 2))
  process.exit(1)
}

console.log(JSON.stringify(handlers[command](), null, 2))
