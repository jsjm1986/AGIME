import React, { useEffect, useState } from 'react';
import { Moon, Sun } from 'lucide-react';
import { cn } from '../../utils';

const getIsDarkMode = (): boolean => {
  const savedUseSystemTheme = localStorage.getItem('use_system_theme');
  if (savedUseSystemTheme === 'true') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  }
  const savedTheme = localStorage.getItem('theme');
  if (savedTheme) {
    return savedTheme === 'dark';
  }
  // Default to dark mode when no preference is saved
  return true;
};

const setThemeMode = (isDark: boolean) => {
  localStorage.setItem('use_system_theme', 'false');
  localStorage.setItem('theme', isDark ? 'dark' : 'light');

  const themeData = {
    mode: isDark ? 'dark' : 'light',
    useSystemTheme: false,
    theme: isDark ? 'dark' : 'light',
  };

  window.electron?.broadcastThemeChange(themeData);

  if (isDark) {
    document.documentElement.classList.add('dark');
    document.documentElement.classList.remove('light');
  } else {
    document.documentElement.classList.remove('dark');
    document.documentElement.classList.add('light');
  }
};

interface ThemeToggleButtonProps {
  className?: string;
}

const ThemeToggleButton: React.FC<ThemeToggleButtonProps> = ({ className }) => {
  const [isDark, setIsDark] = useState(getIsDarkMode);

  useEffect(() => {
    const handleStorageChange = (e: Event & { key?: string | null }) => {
      if (e.key === 'use_system_theme' || e.key === 'theme') {
        setIsDark(getIsDarkMode());
      }
    };

    window.addEventListener('storage', handleStorageChange);
    return () => window.removeEventListener('storage', handleStorageChange);
  }, []);

  useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = () => {
      const savedUseSystemTheme = localStorage.getItem('use_system_theme');
      if (savedUseSystemTheme === 'true') {
        setIsDark(mediaQuery.matches);
      }
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, []);

  const toggleTheme = () => {
    const newIsDark = !isDark;
    setIsDark(newIsDark);
    setThemeMode(newIsDark);
  };

  return (
    <button
      onClick={toggleTheme}
      className={cn(
        "flex items-center justify-center w-8 h-8 rounded-lg",
        "transition-all duration-200 ease-out",
        "text-text-muted hover:text-text-default",
        "hover:bg-black/5 dark:hover:bg-white/10",
        "focus:outline-none focus:ring-2 focus:ring-block-teal/30",
        className
      )}
      title={isDark ? '切换到浅色模式' : '切换到深色模式'}
      aria-label={isDark ? '切换到浅色模式' : '切换到深色模式'}
    >
      {isDark ? (
        <Sun className="w-4 h-4" />
      ) : (
        <Moon className="w-4 h-4" />
      )}
    </button>
  );
};

export default ThemeToggleButton;
