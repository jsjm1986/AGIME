/**
 * Theme initialization script for web build
 * This runs before React loads to prevent flash of wrong theme
 */

function initializeTheme(): void {
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      const useSystemTheme = localStorage.getItem('use_system_theme') === 'true';
      const systemPrefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
      const savedTheme = localStorage.getItem('theme');
      // Default to dark mode when no preference is saved
      const isDark = useSystemTheme ? systemPrefersDark : (savedTheme ? savedTheme === 'dark' : true);

      if (isDark) {
        document.documentElement.classList.add('dark');
        document.documentElement.classList.remove('light');
      } else {
        document.documentElement.classList.remove('dark');
        document.documentElement.classList.add('light');
      }
    }
  } catch (error) {
    console.warn('Failed to initialize theme from localStorage, using dark mode:', error);
    // Keep default dark class
  }
}

// Run immediately
initializeTheme();

// Retry after DOM is ready if initial attempt failed
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => {
    setTimeout(initializeTheme, 50);
  });
}
