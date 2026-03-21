import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import {
  apiClient,
  type SystemAdminIdentity,
  type User,
  type UserPreferences,
} from "../api/client";

type AuthMode = "user" | "system-admin";

interface AuthContextType {
  user: User | null;
  loading: boolean;
  isAdmin: boolean;
  isSystemAdmin: boolean;
  authMode: AuthMode | null;
  login: (apiKey: string) => Promise<void>;
  loginWithPassword: (email: string, password: string) => Promise<void>;
  loginSystemAdmin: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refreshSession: () => Promise<void>;
  updateUserPreferences: (preferences: UserPreferences) => Promise<void>;
  updateUserProfile: (displayName: string) => Promise<void>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

function mapPlatformUser(user: User): User {
  return {
    ...user,
    auth_mode: "user",
  };
}

function mapSystemAdmin(admin: SystemAdminIdentity): User {
  return {
    id: admin.id,
    email: admin.username,
    username: admin.username,
    display_name: admin.display_name,
    role: "system-admin",
    auth_mode: "system-admin",
  };
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  const readPlatformUser = (payload: { user?: User }) => {
    if (!payload?.user) {
      throw new Error("Missing platform user in session payload");
    }
    return payload.user;
  };
  const isSystemAdminPath = () =>
    typeof window !== "undefined" &&
    window.location.pathname.includes("/system-admin");

  const refreshSession = async () => {
    try {
      if (isSystemAdminPath()) {
        const res = await apiClient.getSystemAdminSession();
        setUser(mapSystemAdmin(res.admin));
      } else {
        const res = await apiClient.getSession();
        setUser(mapPlatformUser(readPlatformUser(res as { user?: User })));
      }
    } catch {
      setUser(null);
    }
  };

  useEffect(() => {
    refreshSession().finally(() => setLoading(false));
  }, []);

  const login = async (apiKey: string) => {
    const res = await apiClient.login(apiKey);
    setUser(mapPlatformUser(readPlatformUser(res as { user?: User })));
  };

  const loginWithPassword = async (email: string, password: string) => {
    const res = await apiClient.loginWithPassword(email, password);
    setUser(mapPlatformUser(readPlatformUser(res as { user?: User })));
  };

  const loginSystemAdmin = async (username: string, password: string) => {
    const res = await apiClient.loginSystemAdmin(username, password);
    setUser(mapSystemAdmin(res.admin));
  };

  const logout = async () => {
    if (authMode === "system-admin" || isSystemAdminPath()) {
      await apiClient.logoutSystemAdmin();
    } else {
      await apiClient.logout();
    }
    setUser(null);
  };

  const updateUserPreferences = async (preferences: UserPreferences) => {
    const res = await apiClient.updateMyPreferences(preferences);
    setUser(mapPlatformUser(readPlatformUser(res as { user?: User })));
  };

  const updateUserProfile = async (displayName: string) => {
    const res = await apiClient.updateMyProfile(displayName);
    setUser(mapPlatformUser(readPlatformUser(res as { user?: User })));
    await refreshSession();
  };

  const isSystemAdmin = user?.auth_mode === "system-admin";
  const authMode = user?.auth_mode ?? null;
  const isAdmin = isSystemAdmin;

  return (
    <AuthContext.Provider
      value={{
        user,
        loading,
        isAdmin,
        isSystemAdmin,
        authMode,
        login,
        loginWithPassword,
        loginSystemAdmin,
        logout,
        refreshSession,
        updateUserPreferences,
        updateUserProfile,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
}
