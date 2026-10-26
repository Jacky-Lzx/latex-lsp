#!/usr/bin/env python3
"""Record the real Neovim and Skim windows on macOS, then combine their footage."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
WORK = ROOT / 'target/demos'
PROJECT = WORK / 'synctex-project'
EVIDENCE = WORK / 'synctex-session.json'
SOCKET = '/private/tmp/texquill-synctex.sock'


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def editor(lua):
    expression = 'luaeval(' + json.dumps('(function() ' + lua + ' end)()') + ')'
    return subprocess.check_output(['nvim', '--server', SOCKET, '--remote-expr', expression], text=True)


def prepare():
    if Path(SOCKET).exists():
        raise RuntimeError(f'A demo session already exists at {SOCKET}; close it before preparing another')
    PROJECT.mkdir(parents=True, exist_ok=True)
    shutil.copytree(HERE / 'synctex', PROJECT, dirs_exist_ok=True)
    (PROJECT / '.texquill.json').write_text(json.dumps({'build': {'onSave': False}}))
    EVIDENCE.unlink(missing_ok=True)
    env = dict(os.environ, NVIM_SMOKE_TEST='1', NVIM_TEXQUILL=str(ROOT / 'target/release/texquill'),
               TEXQUILL_SYNCTEX_EVIDENCE=str(EVIDENCE), XDG_STATE_HOME=str(WORK / 'synctex-state'),
               TMPDIR='/private/tmp')
    # Keep XDG_CACHE_HOME: Skim's existing callback uses this shared registry.
    log = (WORK / 'synctex-launch.log').open('w')
    subprocess.Popen(['kitty', '--title', 'TeXquill SyncTeX Demo', '--override', 'remember_window_size=no',
                      '--override', 'initial_window_width=720', '--override', 'initial_window_height=820',
                      '--override', 'font_size=15', '--directory', str(PROJECT),
                      'nvim', '-i', 'NONE', '-n', '--listen', SOCKET, '-S', str(HERE / 'synctex-session.lua'), 'main.tex'],
                     env=env, stdout=log, stderr=log, start_new_session=True)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if EVIDENCE.exists():
            report = json.loads(EVIDENCE.read_text())
            if report.get('error'):
                raise RuntimeError(report['error'])
            if report.get('ready') and (PROJECT / 'build/texquill/main.pdf').exists():
                status = editor("local c = vim.lsp.get_clients({name='texquill'})[1]; "
                                "local r = c:request_sync('workspace/executeCommand', "
                                "{command='latex.buildStatus', arguments={vim.uri_from_fname(vim.fn.getcwd() .. '/main.tex')}}, 1000, 0); "
                                "return r and r.result and r.result.status or 'pending'").strip()
                if status == 'success':
                    print(editor("return require('tex-tools').view()"))
                    print('Ready. Select this PDF in Skim and identify the two window IDs before recording.')
                    return
        time.sleep(0.25)
    raise RuntimeError('Timed out waiting for the Neovim session and compiled PDF')


def capture(args):
    output = ROOT / 'docs/public/demos'
    output.mkdir(parents=True, exist_ok=True)
    before = json.loads(EVIDENCE.read_text())
    # Reset only the disposable demo editor; the PDF is positioned in Skim first.
    editor("vim.cmd('edit sections/results.tex'); vim.api.nvim_win_set_cursor(0, {4, 0}); return true")
    sources = [WORK / f'{args.direction}-{kind}.mov' for kind in ('source', 'pdf')]
    recordings = []
    try:
        for window, path in zip((args.source_window, args.pdf_window), sources):
            recordings.append(subprocess.Popen(['screencapture', '-v', '-V', str(args.seconds),
                                                '-l', str(window), '-k', '-x', str(path)]))
        print(f'Recording {args.direction} for {args.seconds}s.', flush=True)
        if args.direction == 'forward-search':
            time.sleep(3)
            print(editor("return require('tex-tools').forward_search()"), flush=True)
        else:
            print('Now Shift-Command-click the first paragraph on page 2 in Skim.', flush=True)
        for process in recordings:
            if process.wait() != 0:
                raise RuntimeError('Native window capture failed')
    finally:
        for process in recordings:
            if process.poll() is None:
                process.terminate()
    report = json.loads(EVIDENCE.read_text())
    if args.direction == 'forward-search':
        requests = report['requests'][len(before['requests']):]
        forward = [item for item in requests if item['params']['command'] == 'latex.forwardSearch']
        assert forward and not forward[-1].get('error'), 'Forward search did not succeed'
        assert forward[-1]['result']['locations'][0]['page'] == 3, 'Unexpected PDF target page'
    else:
        events = report['inverse'][len(before['inverse']):]
        assert events and events[-1]['file'].endswith('/sections/method.tex'), 'Skim did not navigate to method.tex'
        assert events[-1]['line'] == 4, 'Skim returned an unexpected source line'
        assert report['samples'][-1]['file'] == 'sections/method.tex', 'Editor did not show the returned source'
    render(args.direction, sources, report, args.seconds)


def render(direction, sources, report, seconds):
    output = ROOT / 'docs/public/demos'
    title = ('Forward search  /  Neovim to Skim  /  page 1 to page 3' if direction == 'forward-search'
             else 'Reverse search  /  Skim to Neovim  /  Shift + Command + click')
    caption = WORK / f'{direction}-caption.png'
    run('magick', '-size', '1280x60', 'xc:#181825', '-font', '/System/Library/Fonts/Helvetica.ttc',
        '-pointsize', '24', '-fill', '#cdd6f4', '-gravity', 'west', '-annotate', '+24+0', title, str(caption))
    video = output / f'{direction}.mp4'
    # Both windows are captured live in parallel, including occluded portions.
    # Remove the black window shadow, then crop blank space below the PDF text.
    # The native toolbar and page field remain visible.
    first = WORK / f'{direction}-pdf-first.png'
    run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(sources[1]), '-frames:v', '1', str(first))
    geometry = subprocess.check_output(['magick', str(first), '-threshold', '12%', '-format', '%@', 'info:'], text=True)
    match = re.fullmatch(r'(\d+)x(\d+)\+(\d+)\+(\d+)', geometry.strip())
    assert match, 'Could not identify the PDF window inside its shadow'
    width, height, x, y = map(int, match.groups())
    assert width >= 400 and height >= 600, 'Unexpected PDF window geometry'
    crop = f'crop={width}:{int(height * 0.8) // 2 * 2}:{x}:{y}'
    filters = ('[0:v]fps=20,scale=640:720,setsar=1[left];'
               f'[1:v]{crop},fps=20,'
               'scale=640:720:force_original_aspect_ratio=decrease,'
               'pad=640:720:0:0:color=0x181825,setsar=1[right];'
               '[left][right]hstack=inputs=2:shortest=1,pad=1280:780:0:60:color=0x181825[bg];'
               '[bg][2:v]overlay=0:0')
    run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(sources[0]), '-i', str(sources[1]),
        '-i', str(caption), '-filter_complex', filters, '-c:v', 'libx264', '-crf', '20', '-pix_fmt', 'yuv420p',
        '-movflags', '+faststart', '-an', str(video))
    run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-ss', '0.5', '-i', str(video), '-frames:v', '1', str(output / f'{direction}.png'))
    run('ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', str(video), '-filter_complex',
        'fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle',
        '-loop', '0', str(output / f'{direction}.gif'))
    metadata = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries',
        'stream=codec_name,width,height,pix_fmt:format=duration,size', '-of', 'json', str(video)]))
    stream = metadata['streams'][0]
    assert (stream['codec_name'], stream['width'], stream['height']) == ('h264', 1280, 780)
    assert float(metadata['format']['duration']) >= seconds - 1
    (WORK / f'{direction}-verified.json').write_text(json.dumps({'media': metadata, 'session': report}))
    print(f'Validated {video}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    sub.add_parser('prepare')
    record = sub.add_parser('capture')
    record.add_argument('direction', choices=('forward-search', 'reverse-search'))
    record.add_argument('--source-window', type=int, required=True)
    record.add_argument('--pdf-window', type=int, required=True)
    record.add_argument('--seconds', type=int, default=12)
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    if args.operation == 'prepare':
        prepare()
    else:
        if args.seconds < 8:
            parser.error('Use at least 8 seconds to show both ends of the navigation')
        capture(args)


if __name__ == '__main__':
    main()
