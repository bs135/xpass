#!/usr/bin/env node
// Release version helper (v0.6 plan H10). The version bump happens on `develop`,
// in the release PR, so the release workflow on `main` only tags and builds and
// never pushes a commit to the protected branch.
//
//   node scripts/release-prepare.mjs [x.y.z]   bump all version files (default: next
//                                              version from Conventional Commits)
//   node scripts/release-prepare.mjs --check   version files agree and Cargo.lock is in sync
//   node scripts/release-prepare.mjs --check --newer-than-tag
//                                              ... and the version is above the latest v* tag
//   node scripts/release-prepare.mjs --notes x.y.z
//                                              print the CHANGELOG.md section of x.y.z
//
// No dependencies; needs git and cargo on PATH.

import {execFileSync} from 'node:child_process'
import {readFileSync, writeFileSync} from 'node:fs'
import {dirname, join, resolve} from 'node:path'
import {fileURLToPath} from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const file = (p) => join(root, p)
const read = (p) => readFileSync(file(p), 'utf8')
const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/

function run(cmd, args) {
    return execFileSync(cmd, args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] })
}

function fail(message) {
    console.error(`release-prepare: ${message}`)
    process.exit(1)
}

export function parseVersion(v) {
    const m = SEMVER.exec(v)
    if (!m) fail(`not a release version (x.y.z): ${v}`)
    return m.slice(1).map(Number)
}

export function compare(a, b) {
    const [x, y] = [parseVersion(a), parseVersion(b)]
    for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i]
    return 0
}

/** Bump level from commit messages: 0 none, 1 patch, 2 minor, 3 major. */
export function bumpLevel(messages) {
    let level = 0
    for (const msg of messages) {
        const m = /^(\w+)(?:\([^)]*\))?(!)?:/.exec(msg.trim())
        if (!m) continue
        if (m[2] || /^BREAKING[ -]CHANGE:/m.test(msg)) level = Math.max(level, 3)
        else if (m[1] === 'feat') level = Math.max(level, 2)
        else if (m[1] === 'fix' || m[1] === 'perf') level = Math.max(level, 1)
    }
    return level
}

/** Next version; before 1.0 a breaking change bumps the minor version. */
export function nextVersion(current, level) {
    const [major, minor, patch] = parseVersion(current)
    if (level === 3 && major > 0) return `${major + 1}.0.0`
    if (level >= 2) return `${major}.${minor + 1}.0`
    if (level === 1) return `${major}.${minor}.${patch + 1}`
    return null
}

function latestTag() {
    const tags = run('git', ['tag', '--list', 'v*', '--sort=-v:refname']).split('\n')
    return tags.map((t) => t.trim()).find((t) => SEMVER.test(t.slice(1))) ?? null
}

function commitsSince(tag) {
    const range = tag ? [`${tag}..HEAD`] : ['HEAD']
    const out = run('git', ['log', ...range, '--no-merges', '--format=%B%x1e'])
    return out.split('\x1e').filter((m) => m.trim())
}

/** Versions as written in each file. */
export function versions() {
    const pkg = JSON.parse(read('package.json'))
    const lock = JSON.parse(read('package-lock.json'))
    const toml = /^version = "([^"]+)"/m.exec(read('src-tauri/Cargo.toml'))?.[1]
    const cargoLock = /\[\[package\]\]\r?\nname = "xpass"\r?\nversion = "([^"]+)"/.exec(read('Cargo.lock'))?.[1]
    return {
        'package.json': pkg.version,
        'package-lock.json': lock.version,
        'package-lock.json packages[""]': lock.packages?.['']?.version,
        'src-tauri/Cargo.toml': toml,
        'Cargo.lock (xpass)': cargoLock,
    }
}

function writeJsonVersion(path, version, alsoRootPackage) {
    const data = JSON.parse(read(path))
    data.version = version
    if (alsoRootPackage && data.packages?.['']) data.packages[''].version = version
    writeFileSync(file(path), `${JSON.stringify(data, null, 2)}\n`)
}

function bump(version) {
    writeJsonVersion('package.json', version, false)
    writeJsonVersion('package-lock.json', version, true)
    const toml = read('src-tauri/Cargo.toml')
    writeFileSync(file('src-tauri/Cargo.toml'), toml.replace(/^version = "[^"]+"/m, `version = "${version}"`))
    // Only the workspace members' entries; no dependency upgrades.
    run('cargo', ['update', '--workspace'])
    const changelog = read('CHANGELOG.md')
    if (changelog.includes('## [Unreleased]')) {
        writeFileSync(file('CHANGELOG.md'), changelog.replace('## [Unreleased]', `## [${version}]`))
    } else if (!changelog.includes(`## [${version}]`)) {
        console.warn(`release-prepare: CHANGELOG.md has no "## [Unreleased]" or "## [${version}]" section`)
    }
}

function check(newerThanTag) {
    const found = versions()
    const values = new Set(Object.values(found))
    for (const [where, v] of Object.entries(found)) console.log(`${where.padEnd(32)} ${v}`)
    if (values.size !== 1 || values.has(undefined)) fail('version files disagree')
    const [version] = values
    parseVersion(version)
    // Fails if Cargo.lock still needs changes for the current manifests.
    run('cargo', ['metadata', '--format-version', '1', '--locked', '--no-deps'])
    if (newerThanTag) {
        const tag = latestTag()
        if (tag && compare(version, tag.slice(1)) <= 0) {
            fail(`version ${version} is not above the latest tag ${tag}: run "npm run release:prepare" on develop`)
        }
        console.log(`latest tag ${tag ?? '(none)'} → ${version}`)
    }
    return version
}

function notes(version) {
    const lines = read('CHANGELOG.md').split(/\r?\n/)
    const start = lines.findIndex((l) => l.startsWith(`## [${version}]`))
    if (start < 0) return ''
    const end = lines.findIndex((l, i) => i > start && l.startsWith('## ['))
    return lines
        .slice(start + 1, end < 0 ? undefined : end)
        .join('\n')
        .trim()
}

function main(args) {
    if (args[0] === '--check') {
        console.log(check(args.includes('--newer-than-tag')))
        return
    }
    if (args[0] === '--notes') {
        if (!args[1]) fail('--notes needs a version')
        process.stdout.write(`${notes(args[1])}\n`)
        return
    }
    const current = JSON.parse(read('package.json')).version
    const tag = latestTag()
    let version = args[0]
    if (!version) {
        const level = bumpLevel(commitsSince(tag))
        version = nextVersion(tag ? tag.slice(1) : current, level)
        if (!version) fail(`no feat / fix / perf commit since ${tag ?? 'the start'}: nothing to release`)
    }
    parseVersion(version)
    if (tag && compare(version, tag.slice(1)) <= 0) fail(`${version} is not above the latest tag ${tag}`)
    bump(version)
    check(false)
    console.log(`\nBumped to ${version}. Commit with:\n  git commit -am "chore(release): ${version}"`)
}

// Run only as a script (the unit tests import the helpers).
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main(process.argv.slice(2))
}
