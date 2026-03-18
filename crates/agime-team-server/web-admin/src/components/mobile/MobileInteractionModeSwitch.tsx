import { MessageSquareText, Smartphone } from "lucide-react";
import { Button } from "../ui/button";
import { useMobileInteractionMode } from "../../contexts/MobileInteractionModeContext";

interface MobileInteractionModeSwitchProps {
  className?: string;
}

export function MobileInteractionModeSwitch({
  className = "",
}: MobileInteractionModeSwitchProps) {
  const { effectiveMode, isMobileWorkspace, mode, setMode } =
    useMobileInteractionMode();

  if (!isMobileWorkspace) {
    return null;
  }

  return (
    <div
      className={`inline-flex items-center gap-1 rounded-full border border-border/70 bg-card/85 p-1 shadow-sm ${className}`.trim()}
    >
      <Button
        variant={effectiveMode === "classic" ? "default" : "ghost"}
        size="sm"
        className="h-8 rounded-full px-3"
        onClick={() => setMode("classic")}
        aria-pressed={mode === "classic"}
      >
        <Smartphone className="h-3.5 w-3.5" />
        经典模式
      </Button>
      <Button
        variant={effectiveMode === "conversation" ? "default" : "ghost"}
        size="sm"
        className="h-8 rounded-full px-3"
        onClick={() => setMode("conversation")}
        aria-pressed={mode === "conversation"}
      >
        <MessageSquareText className="h-3.5 w-3.5" />
        对话模式
      </Button>
    </div>
  );
}
