"use strict";

/**
 * lucide-react-native ships a single barrel (`.`) plus a broken `./icons` export
 * (package.json points at dist/{esm,cjs}/icons/index.js, which the published
 * package does not actually contain — see node_modules/lucide-react-native/package.json
 * "exports" and its dist esm/cjs icons directories). Metro has unstable_enablePackageExports
 * on by default (node_modules/metro-config/src/defaults/index.js), so it enforces
 * that map and cannot tree-shake the barrel: importing any icon pulls in the whole
 * 3,490-icon bundle (585,934 bytes / 10.5% of the entry chunk, measured via
 * scripts/ci/bundle_size.py).
 *
 * This rewrites `import { X, Y as Z } from "lucide-react-native"` into one
 * default import per icon, sourced directly from the per-icon dist file by
 * filesystem path. A relative file path bypasses the package's "exports" map
 * entirely (that map only gates bare-specifier subpath resolution — confirmed
 * with `node -e "require.resolve('./node_modules/lucide-react-native/dist/cjs/icons/chevron-right.js')"`
 * succeeding while `require.resolve('lucide-react-native/icons/chevron-right')`
 * throws ERR_PACKAGE_PATH_NOT_EXPORTED), so no metro.config.js resolver override
 * is needed.
 *
 * The icon's exported name does not always match its kebab-case file 1:1 —
 * some names are legacy aliases re-exported from a differently-named file
 * (e.g. AlertTriangle -> triangle-alert.js, MoreHorizontal -> ellipsis.js,
 * Wand2 -> wand-sparkles.js, FileCode2 -> file-code-corner.js). The map below
 * is parsed from the package's own barrel bundle rather than guessed, so
 * renames in a future lucide-react-native version are picked up automatically
 * instead of silently mis-resolving.
 */

const fs = require("fs");
const path = require("path");

let cachedNameToFile = null;

function loadNameToFileMap() {
  if (cachedNameToFile) return cachedNameToFile;

  // Only "." and "./icons" are declared in the package's "exports" map, and
  // "./icons" points at a dist/*/icons/index file that isn't actually shipped
  // (see the file header) — so any subpath request, including
  // require.resolve("lucide-react-native/package.json"), throws
  // ERR_PACKAGE_PATH_NOT_EXPORTED. Resolving the bare specifier is the only
  // subpath the exports map allows; it lands on the CJS barrel, which is
  // parsed below to build the name -> per-icon-file map.
  const bundlePath = require.resolve("lucide-react-native");
  const src = fs.readFileSync(bundlePath, "utf8");
  const iconsDir = path.join(path.dirname(bundlePath), "icons");

  // The barrel looks like:
  //   var triangleAlert = require('./icons/triangle-alert.js');
  //   ...
  //   exports.AlertTriangle = triangleAlert;
  // A name can alias a differently-named file (AlertTriangle is the legacy
  // name for triangle-alert), so both passes are required — the kebab-case
  // guess from the export name alone is not reliable.
  const varToFile = new Map();
  const varDeclRe = /^var (\w+) = require\('\.\/icons\/([a-z0-9-]+)\.js'\);$/gm;
  let match;
  while ((match = varDeclRe.exec(src))) {
    varToFile.set(match[1], match[2]);
  }

  const map = new Map();
  const exportAssignRe = /^exports\.(\w+) = (\w+);$/gm;
  while ((match = exportAssignRe.exec(src))) {
    const [, exportedName, varName] = match;
    const fileBase = varToFile.get(varName);
    if (!fileBase) continue;
    map.set(exportedName, path.join(iconsDir, `${fileBase}.js`));
  }

  cachedNameToFile = map;
  return map;
}

module.exports = function lucideDirectImportPlugin({ types: t }) {
  return {
    name: "lucide-direct-import",
    visitor: {
      ImportDeclaration(importPath, state) {
        const node = importPath.node;
        if (node.source.value !== "lucide-react-native") return;
        if (node.importKind === "type") return;

        const nameToFile = loadNameToFileMap();
        const newImports = [];
        const keptSpecifiers = [];

        for (const specifier of node.specifiers) {
          if (t.isImportNamespaceSpecifier(specifier)) {
            throw importPath.buildCodeFrameError(
              'lucide-direct-import cannot rewrite `import * as X from "lucide-react-native"` — ' +
                "convert it to named imports so the barrel can be tree-shaken.",
            );
          }
          if (t.isImportDefaultSpecifier(specifier)) {
            throw importPath.buildCodeFrameError(
              'lucide-direct-import cannot rewrite a default import from "lucide-react-native" — ' +
                "the package has no default export; use named imports instead.",
            );
          }

          // ImportSpecifier
          if (specifier.importKind === "type") {
            keptSpecifiers.push(specifier);
            continue;
          }

          const importedName = t.isIdentifier(specifier.imported)
            ? specifier.imported.name
            : specifier.imported.value;
          const localName = specifier.local.name;

          const iconFile = nameToFile.get(importedName);
          if (!iconFile) {
            throw importPath.buildCodeFrameError(
              `lucide-direct-import: "${importedName}" is not an icon export of lucide-react-native ` +
                "(checked against the package's own barrel bundle). If this is a new icon after a " +
                "version bump, node_modules/lucide-react-native may need reinstalling.",
            );
          }

          const currentFile = state.file.opts.filename;
          let relative = path
            .relative(path.dirname(currentFile), iconFile)
            .split(path.sep)
            .join("/");
          if (!relative.startsWith(".")) relative = `./${relative}`;

          newImports.push(
            t.importDeclaration(
              [t.importDefaultSpecifier(t.identifier(localName))],
              t.stringLiteral(relative),
            ),
          );
        }

        const replacements = [...newImports];
        if (keptSpecifiers.length > 0) {
          replacements.push(
            t.importDeclaration(
              keptSpecifiers,
              t.stringLiteral("lucide-react-native"),
            ),
          );
        }

        if (replacements.length === 0) {
          importPath.remove();
        } else {
          importPath.replaceWithMultiple(replacements);
        }
      },
    },
  };
};
