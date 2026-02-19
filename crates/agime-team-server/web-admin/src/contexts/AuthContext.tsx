import { createContext, useContext, useState, useEffect, ReactNode } from 'react';
import { apiClient, User } from '../api/client';

interface AuthContextType {
  user: User | null;
  loading: boolean;
  isAdmin: boolean;
  login: (apiKey: string) => Promise<void>;
  loginWithPassword: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refreshSession: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  const refreshSession = async () => {
    try {
      const res = await apiClient.getSession();
      setUser(res.user);
    } catch {
      setUser(null);
    }
  };

  useEffect(() => {
    refreshSession().finally(() => setLoading(false));
  }, []);

  const login = async (apiKey: string) => {
    const res = await apiClient.login(apiKey);
    setUser(res.user);
  };

  const loginWithPassword = async (email: string, password: string) => {
    const res = await apiClient.loginWithPassword(email, password);
    setUser(res.user);
  };

  const logout = async () => {
    await apiClient.logout();
    setUser(null);
  };

  const isAdmin = user?.role === 'admin';

  return (
    <AuthContext.Provider value={{ user, loading, isAdmin, login, loginWithPassword, logout, refreshSession }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
