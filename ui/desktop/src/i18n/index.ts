import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

// Migrate language setting from old key to new key (goose-language -> agime-language)
if (typeof window !== 'undefined' && typeof localStorage !== 'undefined') {
  const oldKey = 'goose-language';
  const newKey = 'agime-language';
  if (!localStorage.getItem(newKey) && localStorage.getItem(oldKey)) {
    localStorage.setItem(newKey, localStorage.getItem(oldKey) as string);
    localStorage.removeItem(oldKey);
  }
}

// Import English translations
import enCommon from './locales/en/common.json';
import enSidebar from './locales/en/sidebar.json';
import enSettings from './locales/en/settings.json';
import enChat from './locales/en/chat.json';
import enGreeting from './locales/en/greeting.json';
import enErrors from './locales/en/errors.json';
import enRecipes from './locales/en/recipes.json';
import enSessions from './locales/en/sessions.json';
import enExtensions from './locales/en/extensions.json';
import enSchedules from './locales/en/schedules.json';
import enModes from './locales/en/modes.json';
import enWelcome from './locales/en/welcome.json';
import enTools from './locales/en/tools.json';

// Import Chinese translations
import zhCommon from './locales/zh-CN/common.json';
import zhSidebar from './locales/zh-CN/sidebar.json';
import zhSettings from './locales/zh-CN/settings.json';
import zhChat from './locales/zh-CN/chat.json';
import zhGreeting from './locales/zh-CN/greeting.json';
import zhErrors from './locales/zh-CN/errors.json';
import zhRecipes from './locales/zh-CN/recipes.json';
import zhSessions from './locales/zh-CN/sessions.json';
import zhExtensions from './locales/zh-CN/extensions.json';
import zhSchedules from './locales/zh-CN/schedules.json';
import zhModes from './locales/zh-CN/modes.json';
import zhWelcome from './locales/zh-CN/welcome.json';
import zhTools from './locales/zh-CN/tools.json';

export const resources = {
  en: {
    common: enCommon,
    sidebar: enSidebar,
    settings: enSettings,
    chat: enChat,
    greeting: enGreeting,
    errors: enErrors,
    recipes: enRecipes,
    sessions: enSessions,
    extensions: enExtensions,
    schedules: enSchedules,
    modes: enModes,
    welcome: enWelcome,
    tools: enTools,
  },
  'zh-CN': {
    common: zhCommon,
    sidebar: zhSidebar,
    settings: zhSettings,
    chat: zhChat,
    greeting: zhGreeting,
    errors: zhErrors,
    recipes: zhRecipes,
    sessions: zhSessions,
    extensions: zhExtensions,
    schedules: zhSchedules,
    modes: zhModes,
    welcome: zhWelcome,
    tools: zhTools,
  },
};

export const supportedLanguages = [
  { code: 'en', name: 'English', nativeName: 'English' },
  { code: 'zh-CN', name: 'Chinese (Simplified)', nativeName: '简体中文' },
];

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    fallbackLng: 'en',
    defaultNS: 'common',
    ns: ['common', 'sidebar', 'settings', 'chat', 'greeting', 'errors', 'recipes', 'sessions', 'extensions', 'schedules', 'modes', 'welcome', 'tools'],
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      lookupLocalStorage: 'agime-language',
    },
  });

export default i18n;
