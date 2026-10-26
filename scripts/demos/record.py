#!/usr/bin/env python3
"""Record real Neovim sessions with the caller's installed configuration."""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SCENES = {
    'completion': ('Search citations by title', '''
Sleep 1s
Type "i"
Type "attention"
Sleep 3s
Tab
Sleep 1s
Escape
Sleep 3s
'''),
    'navigation': ('Navigate and rename across files', '''
Sleep 1s
Type "gd"
Sleep 3s
Ctrl+j
Sleep 2s
Space
Type "rn"
Sleep 1s
Ctrl+u
Type "sec:approach"
Sleep 1s
Enter
Sleep 4s
'''),
    'diagnostics': ('Preview and apply a quick fix', '''
Sleep 1s
Space
Type "d"
Sleep 3s
Escape
Space
Type "a"
Sleep 4s
Enter
Sleep 4s
'''),
    'linked-editing': ('Linked environment editing', '''
Sleep 1s
Type "ciw"
Type "quotation"
Escape
Sleep 3s
Type "u"
Sleep 3s
'''),
    'signatures': ('Your macros, documented in your editor', '''
Sleep 1s
Type "K"
Sleep 4s
Escape
Ctrl+j
Sleep 1s
Type "gK"
Sleep 4s
Escape
Sleep 1s
'''),
    'build': ('Format and build from Neovim', '''
Sleep 1s
Ctrl+j
Sleep 3s
Ctrl+l
Space
Type "lb"
Sleep 8s
'''),
}


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def validate(scene, report):
    if report.get('error'):
        raise RuntimeError(report['error'])
    assert report['client']['name'] == 'texquill'
    samples = report['checkpoints']
    texts = ['\n'.join('\n'.join(lines) for lines in c['buffers'].values()) for c in samples]
    floats = '\n'.join('\n'.join(lines) for c in samples for lines in c['floats'])
    if scene == 'completion':
        assert any(r'\cite{vaswani2017}' in t for t in texts), 'Citation was not accepted'
        assert any((c.get('completion') or {}).get('source') == 'LSP' for c in samples), 'No LSP completion menu'
    elif scene == 'navigation':
        assert any(c['current'] == 'sections/method.tex' for c in samples), 'Definition jump failed'
        final = samples[-1]['buffers']
        assert all('sec:approach' in '\n'.join(final[p]) for p in ('main.tex', 'sections/method.tex')), 'Cross-file rename failed'
        assert 'sec:method' not in texts[-1], 'Old label remains'
    elif scene == 'diagnostics':
        assert any(d.get('code') == 'undefined-reference' for c in samples for d in c['diagnostics']), 'Missing initial warning'
        assert 'sec:metohd' not in texts[-1] and r'\ref{sec:method}' in texts[-1], 'Quick fix failed'
        assert not any(d.get('code') == 'undefined-reference' for d in samples[-1]['diagnostics']), 'Warning did not clear'
    elif scene == 'linked-editing':
        assert any(r'\begin{quotation}' in t and r'\end{quotation}' in t for t in texts), 'Linked edit failed'
        assert r'\begin{quote}' in texts[-1] and r'\end{quote}' in texts[-1], 'Single undo failed'
    elif scene == 'signatures':
        assert 'Highlight a term' in floats and 'Describe a model' in floats, 'Hover/signature documentation missing'
    elif scene == 'build':
        assert any('  A compact' in t for t in texts), 'Formatting did not indent the environment'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scenes', nargs='*', choices=list(SCENES), default=list(SCENES))
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/public/demos')
    parser.add_argument('--server', type=Path, default=ROOT / 'target/release/texquill')
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    for tool in ('nvim', 'vhs', 'ffmpeg', 'ffprobe', 'magick'):
        if not shutil.which(tool):
            parser.error(f'{tool} is required')
    if not args.server.is_file():
        parser.error('Build TeXquill first: cargo build --release --locked')
    with tempfile.TemporaryDirectory(prefix='texquill-demo-', dir='/private/tmp' if Path('/private/tmp').is_dir() else None) as temporary:
        work = Path(temporary)
        for scene in args.scenes:
            title, body = SCENES[scene]
            project = work / scene
            shutil.copytree(HERE / 'fixtures', project)
            (project / '.texquill.json').write_text(json.dumps({'build': {'onSave': scene == 'build'}, 'diagnostics': {'chktex': {'enabled': False}}}))
            if scene == 'diagnostics':
                p = project / 'main.tex'
                p.write_text(p.read_text().replace('ref{sec:method}', 'ref{sec:metohd}'))
            if scene == 'build':
                p = project / 'main.tex'
                p.write_text('\\documentclass{article}\n\\begin{document}\n\\section{A small paper}\n\\begin{quote}\nA compact example with readable source.\n\\end{quote}\n\\end{document}\n')
            entry = 'sections/method.tex' if scene in ('linked-editing', 'signatures') else 'main.tex'
            # Raw UI evidence includes local paths; keep it out of public assets.
            evidence = ROOT / 'target/demos' / f'{scene}.json'
            evidence.parent.mkdir(parents=True, exist_ok=True)
            raw = evidence.parent / f'{scene}-raw.mp4'
            launcher = work / f'{scene}.sh'
            launcher.write_text(f'cd {shlex.quote(str(project))}\nexec nvim -i NONE -n -S {shlex.quote(str(HERE / "session.lua"))} {entry}\n')
            tape = work / f'{scene}.tape'
            tape.write_text(f'''Output "{raw}"
Set Shell "bash"
Set FontFamily "Cascadia Code NF"
Set FontSize 21
Set Width 1280
Set Height 720
Set Padding 12
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set Framerate 20
Hide
Type@1ms "bash {launcher}"
Enter
Sleep 6s
Show
{body}
Hide
Type@1ms ":qa!"
Enter
''')
            env = dict(os.environ, NVIM_SMOKE_TEST='1', NVIM_TEXQUILL=str(args.server.resolve()),
                       TEXQUILL_DEMO_SCENE=scene, TEXQUILL_DEMO_EVIDENCE=str(evidence),
                       XDG_CACHE_HOME=str(work / 'cache'), XDG_STATE_HOME=str(work / 'state'),
                       TMPDIR=str(work))
            print(f'Recording {scene} with {os.environ.get("XDG_CONFIG_HOME", "~/.config")}/nvim', flush=True)
            run('vhs', str(tape), env=env)
            report = json.loads(evidence.read_text())
            validate(scene, report)
            if scene == 'build':
                assert list(project.rglob('main.pdf')), 'Build did not produce a PDF'
            caption = work / 'caption.png'
            run('magick', '-size', '1280x60', 'xc:#181825', '-font', '/System/Library/Fonts/Helvetica.ttc', '-pointsize', '24',
                '-fill', '#cdd6f4', '-gravity', 'west', '-annotate', '+24+0', 'TeXquill  /  ' + title, str(caption))
            video = args.output / f'{scene}.mp4'
            run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(raw),
                '-i', str(caption), '-filter_complex', '[0:v]pad=iw:ih+60:0:60:color=0x181825[bg];[bg][1:v]overlay=0:0',
                '-c:v', 'libx264', '-crf', '20', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', '-an', str(video))
            media = json.loads(subprocess.check_output([
                'ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_entries',
                'stream=codec_name,width,height,pix_fmt:format=duration,size', '-of', 'json', str(video),
            ]))
            stream = media['streams'][0]
            assert (stream['codec_name'], stream['width'], stream['height'], stream['pix_fmt']) == ('h264', 1280, 780, 'yuv420p')
            assert float(media['format']['duration']) > 0
            report['export'] = media
            evidence.write_text(json.dumps(report))
            run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-ss', '0.5', '-i', str(video), '-frames:v', '1', str(args.output / f'{scene}.png'))
            run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(video),
                '-filter_complex', 'fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle',
                '-loop', '0', str(args.output / f'{scene}.gif'))
            print(f'Validated {scene}: {video}', flush=True)


if __name__ == '__main__':
    main()
