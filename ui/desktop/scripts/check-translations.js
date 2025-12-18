#!/usr/bin/env node

/**
 * Translation Key Consistency Checker
 *
 * This script checks that all translation files have consistent keys across all languages.
 * Run this script to ensure no translation keys are missing when adding new features.
 *
 * Usage: node scripts/check-translations.js
 */

const fs = require('fs');
const path = require('path');

const LOCALES_DIR = path.join(__dirname, '../src/i18n/locales');
const SUPPORTED_LANGUAGES = ['en', 'zh-CN'];
const NAMESPACES = ['common', 'chat', 'errors', 'greeting', 'settings', 'sidebar', 'recipes', 'sessions', 'extensions', 'schedules'];

function getAllKeys(obj, prefix = '') {
  let keys = [];
  for (const key in obj) {
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (typeof obj[key] === 'object' && obj[key] !== null) {
      keys = keys.concat(getAllKeys(obj[key], fullKey));
    } else {
      keys.push(fullKey);
    }
  }
  return keys;
}

function loadTranslationFile(lang, namespace) {
  const filePath = path.join(LOCALES_DIR, lang, `${namespace}.json`);
  if (!fs.existsSync(filePath)) {
    return null;
  }
  try {
    const content = fs.readFileSync(filePath, 'utf-8');
    return JSON.parse(content);
  } catch (error) {
    console.error(`Error reading ${filePath}: ${error.message}`);
    return null;
  }
}

function checkTranslations() {
  let hasErrors = false;
  const results = {
    missing: {},
    extra: {},
  };

  // Use English as the reference language
  const referenceLang = 'en';

  for (const namespace of NAMESPACES) {
    const referenceData = loadTranslationFile(referenceLang, namespace);
    if (!referenceData) {
      console.error(`Missing reference file: ${referenceLang}/${namespace}.json`);
      hasErrors = true;
      continue;
    }

    const referenceKeys = getAllKeys(referenceData);

    for (const lang of SUPPORTED_LANGUAGES) {
      if (lang === referenceLang) continue;

      const langData = loadTranslationFile(lang, namespace);
      if (!langData) {
        console.error(`Missing translation file: ${lang}/${namespace}.json`);
        hasErrors = true;
        continue;
      }

      const langKeys = getAllKeys(langData);

      // Find missing keys (in reference but not in this language)
      const missingKeys = referenceKeys.filter(key => !langKeys.includes(key));
      if (missingKeys.length > 0) {
        hasErrors = true;
        if (!results.missing[lang]) results.missing[lang] = {};
        results.missing[lang][namespace] = missingKeys;
      }

      // Find extra keys (in this language but not in reference)
      const extraKeys = langKeys.filter(key => !referenceKeys.includes(key));
      if (extraKeys.length > 0) {
        if (!results.extra[lang]) results.extra[lang] = {};
        results.extra[lang][namespace] = extraKeys;
      }
    }
  }

  // Print results
  console.log('\n=== Translation Key Check Results ===\n');

  if (Object.keys(results.missing).length > 0) {
    console.log('MISSING KEYS (need to be added):');
    console.log('--------------------------------');
    for (const lang in results.missing) {
      console.log(`\n[${lang}]`);
      for (const namespace in results.missing[lang]) {
        console.log(`  ${namespace}.json:`);
        for (const key of results.missing[lang][namespace]) {
          console.log(`    - ${key}`);
        }
      }
    }
  }

  if (Object.keys(results.extra).length > 0) {
    console.log('\nEXTRA KEYS (may be unused or reference is missing):');
    console.log('---------------------------------------------------');
    for (const lang in results.extra) {
      console.log(`\n[${lang}]`);
      for (const namespace in results.extra[lang]) {
        console.log(`  ${namespace}.json:`);
        for (const key of results.extra[lang][namespace]) {
          console.log(`    - ${key}`);
        }
      }
    }
  }

  if (!hasErrors && Object.keys(results.extra).length === 0) {
    console.log('All translation keys are consistent across all languages!');
  }

  console.log('\n');
  return hasErrors ? 1 : 0;
}

process.exit(checkTranslations());
