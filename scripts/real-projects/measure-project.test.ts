import path from 'node:path';

import type { SpawnSyncReturns } from 'node:child_process';

import { expect, test } from 'vitest';

import {
  type MeasuredCommandResult,
  outputPathsForProject,
  parseArgs,
  parsePeakFootprintBytes,
  parsePeakRssBytes,
  parseRustJobs,
  peakRssMb,
  projectNameFromPath,
  resolveProject,
  runMeasuredCommand,
  slugify,
  timeMeasurementForPlatform,
} from './measure-project';

const MACOS_TIME_REPORT = [
  '        0.55 real         0.42 user         0.12 sys',
  '           170999808  maximum resident set size',
  '                   0  average shared memory size',
  '               10712  page reclaims',
  '           167330320  peak memory footprint',
].join('\n');

const LINUX_TIME_REPORT = [
  '\tCommand being timed: "target/release/surge --project tsconfig.json"',
  '\tUser time (seconds): 0.40',
  '\tMaximum resident set size (kbytes): 166992',
  '\tExit status: 0',
].join('\n');

function fakeSpawn(
  overrides: Partial<SpawnSyncReturns<string>> = {},
): { spawn: typeof import('node:child_process').spawnSync; calls: Array<{ command: string; args: string[] }> } {
  const calls: Array<{ command: string; args: string[] }> = [];
  const spawn = ((command: string, args: string[]) => {
    calls.push({ command, args });
    return {
      pid: 1,
      output: [],
      stdout: '',
      stderr: '',
      status: 0,
      signal: null,
      ...overrides,
    } as SpawnSyncReturns<string>;
  }) as unknown as typeof import('node:child_process').spawnSync;
  return { spawn, calls };
}

test('parsePeakRssBytes reads macOS time -l bytes verbatim', () => {
  expect(parsePeakRssBytes(MACOS_TIME_REPORT, 'macos-time')).toBe(170999808);
});

test('parsePeakRssBytes converts Linux time -v kbytes to bytes', () => {
  expect(parsePeakRssBytes(LINUX_TIME_REPORT, 'linux-time')).toBe(166992 * 1024);
});

test('parsePeakRssBytes returns null when the field is absent', () => {
  expect(parsePeakRssBytes('no rss here', 'macos-time')).toBe(null);
  expect(parsePeakRssBytes('no rss here', 'linux-time')).toBe(null);
  expect(parsePeakRssBytes(MACOS_TIME_REPORT, 'unavailable')).toBe(null);
});

test('parsePeakFootprintBytes reads the macOS phys_footprint peak', () => {
  expect(parsePeakFootprintBytes(MACOS_TIME_REPORT, 'macos-time')).toBe(167330320);
});

test('parsePeakFootprintBytes is macOS-only and null when absent', () => {
  expect(parsePeakFootprintBytes(LINUX_TIME_REPORT, 'linux-time')).toBe(null);
  expect(parsePeakFootprintBytes(MACOS_TIME_REPORT, 'unavailable')).toBe(null);
  expect(parsePeakFootprintBytes('no footprint here', 'macos-time')).toBe(null);
});

test('peakRssMb rounds bytes to one decimal megabyte', () => {
  expect(peakRssMb(null)).toBe(null);
  expect(peakRssMb(1024 * 1024)).toBe(1);
  expect(peakRssMb(170999808)).toBe(163.1);
});

test('timeMeasurementForPlatform maps darwin/linux and rejects others', () => {
  expect(timeMeasurementForPlatform('darwin')).toStrictEqual({ flag: '-l', source: 'macos-time' });
  expect(timeMeasurementForPlatform('linux')).toStrictEqual({ flag: '-v', source: 'linux-time' });
  expect(timeMeasurementForPlatform('win32')).toBe(null);
});

test('runMeasuredCommand parses macOS peak RSS and keeps child output clean', () => {
  const { spawn, calls } = fakeSpawn({ stdout: 'OK', stderr: 'Timings:\n  parsing: 1ms' });
  const result: MeasuredCommandResult = runMeasuredCommand('bin', ['--project', 'tsconfig.json'], {
    platform: 'darwin',
    spawn,
    timeBinaryExists: () => true,
    timeBinaryPath: '/usr/bin/time',
    makeReportPath: () => '/tmp/report',
    readReport: () => MACOS_TIME_REPORT,
    now: () => 0,
  });

  expect(result.peakRssBytes).toBe(170999808);
  expect(result.peakRssSource).toBe('macos-time');
  expect(result.peakFootprintBytes).toBe(167330320);
  expect(result.status).toBe(0);
  expect(result.stdout).toBe('OK');
  expect(result.stderr).toBe('Timings:\n  parsing: 1ms');
  // time is the spawned process; the measured command is passed after `-o file`.
  expect(calls[0]).toStrictEqual({
    command: '/usr/bin/time',
    args: ['-l', '-o', '/tmp/report', 'bin', '--project', 'tsconfig.json'],
  });
});

test('runMeasuredCommand parses Linux peak RSS', () => {
  const { spawn, calls } = fakeSpawn();
  const result = runMeasuredCommand('bin', ['--project', 'tsconfig.json'], {
    platform: 'linux',
    spawn,
    timeBinaryExists: () => true,
    makeReportPath: () => '/tmp/report',
    readReport: () => LINUX_TIME_REPORT,
  });

  expect(result.peakRssBytes).toBe(166992 * 1024);
  expect(result.peakRssSource).toBe('linux-time');
  expect(result.peakFootprintBytes).toBe(null);
  expect(calls[0].args[0]).toBe('-v');
});

test('runMeasuredCommand falls back to direct execution when time is unavailable', () => {
  const { spawn, calls } = fakeSpawn({ stdout: 'direct', stderr: '' });
  const result = runMeasuredCommand('bin', ['--project', 'tsconfig.json'], {
    platform: 'darwin',
    spawn,
    timeBinaryExists: () => false,
  });

  expect(result.peakRssBytes).toBe(null);
  expect(result.peakRssSource).toBe('unavailable');
  expect(result.stdout).toBe('direct');
  // No /usr/bin/time wrapper: the command runs directly.
  expect(calls[0]).toStrictEqual({ command: 'bin', args: ['--project', 'tsconfig.json'] });
});

test('runMeasuredCommand still reports memory when the child command fails', () => {
  const { spawn } = fakeSpawn({ status: 2, stdout: 'partial', stderr: 'boom' });
  const result = runMeasuredCommand('bin', ['--project', 'tsconfig.json'], {
    platform: 'darwin',
    spawn,
    timeBinaryExists: () => true,
    makeReportPath: () => '/tmp/report',
    readReport: () => MACOS_TIME_REPORT,
  });

  expect(result.status).toBe(2);
  expect(result.stdout).toBe('partial');
  expect(result.stderr).toBe('boom');
  expect(result.peakRssBytes).toBe(170999808);
  expect(result.peakRssSource).toBe('macos-time');
});

test('runMeasuredCommand marks memory unavailable when the report is unparseable', () => {
  const { spawn } = fakeSpawn();
  const result = runMeasuredCommand('bin', [], {
    platform: 'darwin',
    spawn,
    timeBinaryExists: () => true,
    makeReportPath: () => '/tmp/report',
    readReport: () => 'garbage with no rss',
  });

  expect(result.peakRssBytes).toBe(null);
  expect(result.peakRssSource).toBe('unavailable');
});

test('parseArgs reads all supported flags', () => {
  const parsed = parseArgs([
    '--project',
    '/abs/project/tsconfig.json',
    '--name',
    'My App',
    '--maxDiagnostics',
    '1000',
    '--rustJobs',
    '1,4',
    '--outDir',
    '/tmp/out',
    '--allowMissing',
  ]);

  expect(parsed.project).toBe('/abs/project/tsconfig.json');
  expect(parsed.name).toBe('My App');
  expect(parsed.maxDiagnostics).toBe(1000);
  expect(parsed.rustJobs).toStrictEqual([1, 4]);
  expect(parsed.outDir).toBe('/tmp/out');
  expect(parsed.allowMissing).toBe(true);
});

test('parseArgs applies defaults', () => {
  const parsed = parseArgs(['--project', '/abs/project']);
  expect(parsed.maxDiagnostics).toBe(500);
  expect(parsed.rustJobs).toStrictEqual([1, 'auto']);
  expect(parsed.outDir).toBe(null);
  expect(parsed.name).toBe(null);
  expect(parsed.allowMissing).toBe(false);
});

test('parseArgs rejects unknown arguments and bad values', () => {
  expect(() => parseArgs(['--nope'])).toThrow(/Unknown argument/);
  expect(() => parseArgs(['--maxDiagnostics', '0'])).toThrow(/positive integer/);
  expect(() => parseArgs(['--project'])).toThrow(/Missing value/);
});

test('parseRustJobs parses, validates, and dedupes', () => {
  expect(parseRustJobs('1,4')).toStrictEqual([1, 4]);
  expect(parseRustJobs('1,2,4')).toStrictEqual([1, 2, 4]);
  expect(parseRustJobs(' 1 , 1 , 4 ')).toStrictEqual([1, 4]);
  expect(() => parseRustJobs('')).toThrow(/positive integers/);
  expect(() => parseRustJobs('1,foo')).toThrow(/positive integers/);
  expect(() => parseRustJobs('0,1')).toThrow(/positive integers/);
});

test('resolveProject uses an explicit tsconfig file path', () => {
  const resolved = resolveProject(
    { project: '/abs/project/tsconfig.json' },
    { workspaceRoot: '/repo', classify: (p) => (p === '/abs/project/tsconfig.json' ? 'file' : 'missing') },
  );
  expect(resolved).toStrictEqual({
    root: '/abs/project',
    tsconfig: '/abs/project/tsconfig.json',
    attempted: ['/abs/project/tsconfig.json'],
  });
});

test('resolveProject finds tsconfig.json inside a directory', () => {
  const tsconfig = path.join('/abs/project', 'tsconfig.json');
  const resolved = resolveProject(
    { project: '/abs/project' },
    {
      workspaceRoot: '/repo',
      classify: (p) => {
        if (p === '/abs/project') return 'dir';
        if (p === tsconfig) return 'file';
        return 'missing';
      },
    },
  );
  expect(resolved?.root).toBe('/abs/project');
  expect(resolved?.tsconfig).toBe(tsconfig);
});

test('resolveProject returns null when directory has no tsconfig', () => {
  const resolved = resolveProject(
    { project: '/abs/project' },
    { workspaceRoot: '/repo', classify: (p) => (p === '/abs/project' ? 'dir' : 'missing') },
  );
  expect(resolved).toBe(null);
});

test('resolveProject resolves relative project paths against cwd', () => {
  const tsconfig = path.resolve('/work', 'sub', 'tsconfig.json');
  const resolved = resolveProject(
    { project: 'sub/tsconfig.json' },
    {
      workspaceRoot: '/repo',
      cwd: '/work',
      classify: (p) => (p === tsconfig ? 'file' : 'missing'),
    },
  );
  expect(resolved?.tsconfig).toBe(tsconfig);
  expect(resolved?.root).toBe(path.dirname(tsconfig));
});

test('resolveProject returns null without a project', () => {
  expect(resolveProject({ project: null }, { workspaceRoot: '/repo' })).toBe(null);
});

test('projectNameFromPath and slugify derive stable slugs', () => {
  expect(projectNameFromPath('/abs/My Next App')).toBe('my-next-app');
  expect(slugify('  trpc  ')).toBe('trpc');
  expect(slugify('@scope/pkg')).toBe('scope-pkg');
  expect(slugify('***')).toBe('project');
});

test('outputPathsForProject names artifacts per rustJobs', () => {
  const outputs = outputPathsForProject('/out/my-app', [1, 2, 4]);
  expect(outputs.measurementMd).toBe(path.join('/out/my-app', 'measurement.md'));
  expect(outputs.oracleCompareTxt).toBe(path.join('/out/my-app', 'oracle-compare.txt'));
  expect(outputs.oracleCompareJson).toBe(path.join('/out/my-app', 'oracle-compare.json'));
  expect(outputs.compatReportJson).toBe(path.join('/out/my-app', 'compat-report.json'));
  expect(outputs.timingsTxt).toBe(path.join('/out/my-app', 'timings.txt'));
  expect(
    outputs.jobs.map((job) => [job.jobs, path.basename(job.json), path.basename(job.svg)]),
  ).toStrictEqual([
    [1, 'jobs1.json', 'jobs1.svg'],
    [2, 'jobs2.json', 'jobs2.svg'],
    [4, 'jobs4.json', 'jobs4.svg'],
  ]);
});
