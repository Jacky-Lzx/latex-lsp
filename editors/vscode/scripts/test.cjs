const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');

async function executablePath() {
  if (process.env.VSCODE_EXECUTABLE_PATH) return process.env.VSCODE_EXECUTABLE_PATH;
  const downloaded = await downloadAndUnzipVSCode();
  // Newer macOS releases renamed Electron to Code. Older test-electron
  // versions still return the former name after downloading the application.
  if (process.platform === 'darwin') {
    const renamed = path.join(path.dirname(downloaded), 'Code');
    try { await fs.access(renamed); return renamed; } catch { /* Older release. */ }
  }
  return downloaded;
}

async function main() {
  const extensionDevelopmentPath = path.resolve(__dirname, '..');
  const testNeovim = process.argv.includes('--neovim');
  const neovimExtension = process.env.TEXQUILL_TEST_NEOVIM_EXTENSION;
  if (testNeovim) {
    if (!neovimExtension) throw new Error('Set TEXQUILL_TEST_NEOVIM_EXTENSION to an installed VSCode Neovim extension directory.');
    const manifest = JSON.parse(await fs.readFile(path.join(neovimExtension, 'package.json'), 'utf8'));
    if (`${manifest.publisher}.${manifest.name}` !== 'asvetliakov.vscode-neovim') throw new Error('Expected the VSCode Neovim extension.');
  }
  const server = process.env.TEXQUILL_TEST_SERVER
    ?? path.resolve(extensionDevelopmentPath, '../../target/release/texquill');
  await fs.access(server);
  // realpath avoids /var vs /private/var URI aliases on macOS.
  const temporaryRoot = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'texquill-vscode-')));
  const workspace = path.join(temporaryRoot, 'project with spaces');
  const userData = path.join(temporaryRoot, 'user-data');
  try {
    await fs.mkdir(workspace);
    await fs.mkdir(path.join(userData, 'User'), { recursive: true });
    await fs.writeFile(path.join(workspace, 'main.tex'), [
      '\\documentclass{article}',
      '\\begin{document}',
      '\\input{chapter}',
      'See \\ref{sec:intro}.',
      '\\newpage',
      'Second page.',
      '\\end{document}',
      '',
    ].join('\n'));
    await fs.writeFile(path.join(workspace, 'chapter.tex'), '\\section{Introduction}\n\\label{sec:intro}\nHello.\n');
    await fs.writeFile(path.join(workspace, 'references.bib'), '@book{example,\n title={Example},\n author={Author},\n year={2026}\n}\n');
    await fs.writeFile(path.join(workspace, 'slow.cjs'),
      'require("node:fs").writeFileSync(require("node:path").join(__dirname, "slow.pid"), String(process.pid)); setTimeout(() => {}, 60000);');
    await fs.writeFile(path.join(userData, 'User/settings.json'), JSON.stringify({
      ...(testNeovim ? {
        'vscode-neovim.neovimClean': true,
        [`vscode-neovim.neovimExecutablePaths.${process.platform}`]: process.env.TEXQUILL_TEST_NEOVIM ?? 'nvim',
        'extensions.experimental.affinity': { 'asvetliakov.vscode-neovim': 1 },
      } : {}),
      'texquill.serverPath': server,
      'texquill.settings': { distribution: { enabled: false }, build: { executable: 'pdflatex', outputDirectory: 'build' } },
      'security.workspace.trust.enabled': false,
      'workbench.startupEditor': 'none',
      'extensions.autoUpdate': false,
      'extensions.autoCheckUpdates': false,
    }));
    // Codex/T3 may itself run under Electron's Node mode. A child VS Code must
    // launch as a GUI application, not inherit that mode.
    delete process.env.ELECTRON_RUN_AS_NODE;
    const options = {
      extensionDevelopmentPath: testNeovim ? [extensionDevelopmentPath, neovimExtension] : extensionDevelopmentPath,
      extensionTestsPath: testNeovim ? path.join(extensionDevelopmentPath, 'dist/test/neovim.js')
        : path.join(extensionDevelopmentPath, 'dist/test/integration.js'),
      vscodeExecutablePath: await executablePath(),
      extensionTestsEnv: {
        TEXQUILL_TEST_WORKSPACE: workspace,
        TEXQUILL_TEST_NODE: process.execPath,
      },
      launchArgs: [workspace, '--user-data-dir', userData, '--extensions-dir', path.join(temporaryRoot, 'extensions'),
        '--disable-extensions',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--no-sandbox'],
    };
    await runTests(options);
  } finally {
    await fs.rm(temporaryRoot, { recursive: true, force: true });
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
