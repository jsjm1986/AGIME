import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { LanguageSwitcher } from "../components/LanguageSwitcher";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { extractInviteCode } from "../utils/navigation";

export function JoinEntryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [value, setValue] = useState("");
  const [error, setError] = useState("");

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const code = extractInviteCode(value);
    if (!code) {
      setError(
        t("join.entryInvalid", {
          defaultValue: "请输入有效的邀请链接或邀请代码。",
        }),
      );
      return;
    }
    navigate(`/join/${code}`);
  };

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,hsl(var(--primary))/0.08,transparent_28%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--ui-shell-gradient-end)))] px-4 py-8">
      <div className="absolute top-4 right-4">
        <LanguageSwitcher />
      </div>
      <div className="mx-auto flex min-h-[80vh] w-full max-w-5xl items-center justify-center">
        <Card className="w-full max-w-md border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--card))/0.92] shadow-[0_26px_54px_hsl(var(--ui-shadow)/0.12)]">
          <CardHeader>
            <CardTitle className="text-[26px]">
              {t("join.entryTitle", { defaultValue: "打开邀请链接" })}
            </CardTitle>
            <CardDescription className="leading-6">
              {t("join.entryDescription", {
                defaultValue:
                  "粘贴完整邀请链接或邀请代码，系统会自动识别并进入加入团队流程。",
              })}
            </CardDescription>
          </CardHeader>
          <form onSubmit={handleSubmit}>
            <CardContent className="space-y-4">
              {error && (
                <div className="rounded-[14px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.96] p-3 text-sm text-[hsl(var(--status-error-text))]">
                  {error}
                </div>
              )}
              <div className="space-y-2">
                <label className="text-sm font-medium">
                  {t("join.entryLabel", {
                    defaultValue: "邀请链接或邀请代码",
                  })}
                </label>
                <Input
                  type="text"
                  value={value}
                  onChange={(e) => {
                    setValue(e.target.value);
                    if (error) {
                      setError("");
                    }
                  }}
                  placeholder={t("join.entryPlaceholder", {
                    defaultValue:
                      "例如 http://117.72.54.28:9999/join/xxxx 或直接粘贴代码",
                  })}
                  required
                />
              </div>
            </CardContent>
            <CardFooter>
              <Button type="submit" className="w-full">
                {t("join.entrySubmit", { defaultValue: "继续加入团队" })}
              </Button>
            </CardFooter>
          </form>
        </Card>
      </div>
    </div>
  );
}
