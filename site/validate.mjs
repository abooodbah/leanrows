#!/usr/bin/env node

import { readFile, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const siteRoot = dirname(fileURLToPath(import.meta.url));
const indexPath = resolve(siteRoot, "index.html");
const cssPath = resolve(siteRoot, "styles.css");
const html = await readFile(indexPath, "utf8");
const css = await readFile(cssPath, "utf8");
const failures = [];

for (const required of [
  ".nojekyll",
  "index.html",
  "robots.txt",
  "sitemap.xml",
  "styles.css",
  "assets/leanrows-app.png",
  "assets/leanrows-icon.png",
  "assets/leanrows-icon.svg",
]) {
  try {
    const entry = await stat(resolve(siteRoot, required));
    if (!entry.isFile()) failures.push(`required path is not a file: ${required}`);
  } catch {
    failures.push(`missing required site file: ${required}`);
  }
}

function requireMatch(value, pattern, message) {
  if (!pattern.test(value)) failures.push(message);
}

requireMatch(html, /^<!doctype html>/i, "index.html must start with an HTML5 doctype");
requireMatch(html, /<html\s+lang="en">/i, "the document language must be declared");
requireMatch(html, /<meta\s+name="viewport"/i, "the viewport meta tag is required");
requireMatch(html, /<meta\s+name="description"/i, "a meta description is required");
requireMatch(html, /<script\s+type="application\/ld\+json">/i, "SoftwareApplication JSON-LD is required");
requireMatch(html, /assets\/leanrows-icon\.png/i, "the social preview must use the PNG icon");
requireMatch(html, /<figure\s+class="app-capture">/i, "a real application-capture figure is required");
requireMatch(html, /src="assets\/leanrows-app\.png"/i, "the real application capture must be rendered");
requireMatch(html, /synthetic data/i, "the application capture must be identified as synthetic data");
requireMatch(
  html,
  /LeanRows v0\.1\.2 places the virtual owner-data grid/i,
  "the current interface description must identify v0.1.2 and its owner-data grid",
);
requireMatch(
  html,
  /responsive\s+top bar/i,
  "the current interface description must identify the responsive top bar",
);
requireMatch(
  html,
  /inline search/i,
  "the current interface description must identify inline search",
);
requireMatch(
  html,
  /System and Light/i,
  "the current interface description must identify the supported appearance choices",
);
requireMatch(
  html,
  /Windows high contrast uses system colors/i,
  "the current interface description must identify the high-contrast fallback",
);
requireMatch(html, /<main\s+id="main-content">/i, "a named main landmark is required");
requireMatch(css, /:focus-visible\s*\{[^}]*outline:/s, "visible focus styles are required");
requireMatch(css, /prefers-reduced-motion:\s*reduce/i, "reduced-motion support is required");
requireMatch(css, /@media\s*\(max-width:\s*48rem\)/i, "mobile layout rules are required");
requireMatch(css, /@media\s*\(max-width:\s*64rem\)/i, "tablet layout rules are required");

const structuredDataMatch = html.match(
  /<script\s+type="application\/ld\+json">\s*([\s\S]*?)\s*<\/script>/i,
);
if (structuredDataMatch) {
  try {
    const structuredData = JSON.parse(structuredDataMatch[1]);
    if (structuredData["@type"] !== "SoftwareApplication") {
      failures.push("JSON-LD must describe a SoftwareApplication");
    }
    if (structuredData.name !== "LeanRows" || structuredData.softwareVersion !== "0.1.2") {
      failures.push("JSON-LD name and release version must match the product");
    }
    if (structuredData.downloadUrl !== "https://github.com/abooodbah/leanrows/releases/latest") {
      failures.push("JSON-LD downloadUrl must use the latest GitHub release");
    }
  } catch (error) {
    failures.push(`JSON-LD must contain valid JSON: ${error.message}`);
  }
}

const h1Count = (html.match(/<h1\b/gi) ?? []).length;
if (h1Count !== 1) failures.push(`expected exactly one h1; found ${h1Count}`);

try {
  const capture = await readFile(resolve(siteRoot, "assets/leanrows-app.png"));
  const pngSignature = "89504e470d0a1a0a";
  if (capture.length < 24 || capture.subarray(0, 8).toString("hex") !== pngSignature) {
    failures.push("the application capture must be a valid PNG file");
  } else {
    const captureWidth = capture.readUInt32BE(16);
    const captureHeight = capture.readUInt32BE(20);
    if (captureWidth !== 1180 || captureHeight !== 720) {
      failures.push(
        `the application capture must remain 1180x720; found ${captureWidth}x${captureHeight}`,
      );
    }
  }
} catch (error) {
  failures.push(`the application capture could not be inspected: ${error.message}`);
}

const primaryCount = (html.match(/class="primary-action"/g) ?? []).length;
if (primaryCount !== 1) {
  failures.push(`expected exactly one primary release action; found ${primaryCount}`);
}

const latestRelease = "https://github.com/abooodbah/leanrows/releases/latest";
if (!html.includes(`href="${latestRelease}"`)) {
  failures.push("the primary action must point to GitHub releases/latest");
}

for (const forbidden of ["TODO", "TBD", "lorem ipsum", "placeholder", "customer logo", "testimonial"]) {
  if (html.toLowerCase().includes(forbidden.toLowerCase())) {
    failures.push(`shipped copy contains forbidden placeholder phrase: ${forbidden}`);
  }
}

const ids = new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]));
for (const match of html.matchAll(/href="#([^"]+)"/g)) {
  if (!ids.has(match[1])) failures.push(`same-page link has no target: #${match[1]}`);
}

for (const match of html.matchAll(/(?:href|src)="([^"#]+)"/g)) {
  const value = match[1];
  if (/^(?:https?:|mailto:|data:)/i.test(value)) continue;
  const localPath = resolve(siteRoot, value.split(/[?#]/, 1)[0]);
  if (!localPath.startsWith(siteRoot)) {
    failures.push(`local reference escapes site/: ${value}`);
    continue;
  }
  try {
    const entry = await stat(localPath);
    if (!entry.isFile()) failures.push(`local reference is not a file: ${value}`);
  } catch {
    failures.push(`missing local reference: ${value}`);
  }
}

const externalHosts = new Set(
  [...html.matchAll(/(?:href|src)="(https?:\/\/[^"/]+)/g)].map((match) => match[1]),
);
for (const host of externalHosts) {
  if (host !== "https://github.com" && host !== "https://abooodbah.github.io") {
    failures.push(`unexpected external host: ${host}`);
  }
}

if (failures.length > 0) {
  console.error("LeanRows site validation failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log("LeanRows site validation passed (semantic, link, asset, and responsive contracts)." );
}
