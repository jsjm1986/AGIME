import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { Button } from "../ui/button";

interface BottomSheetPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  children: ReactNode;
  fullHeight?: boolean;
  onBack?: () => void;
  backLabel?: string;
}

export function BottomSheetPanel({
  open,
  onOpenChange,
  title,
  description,
  children,
  fullHeight = false,
  onBack,
  backLabel = "返回",
}: BottomSheetPanelProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className={`inset-x-0 bottom-0 top-auto flex min-h-0 w-screen max-w-none flex-col translate-x-0 translate-y-0 gap-0 overflow-hidden rounded-t-[24px] rounded-b-none border-x-0 border-b-0 px-0 pb-0 pt-0 data-[state=closed]:slide-out-to-bottom data-[state=open]:slide-in-from-bottom sm:inset-x-auto sm:bottom-auto sm:left-[50%] sm:top-[50%] sm:max-w-lg sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-[24px] sm:border ${fullHeight ? 'max-h-[92dvh]' : 'max-h-[78dvh]'}`}>
        <div className="mx-auto mt-2.5 h-1.5 w-10 rounded-full bg-border/75" />
        <DialogHeader className="border-b border-border/55 px-4 py-3 text-left">
          <div className="flex items-start gap-3 pr-10">
            {onBack ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-8 shrink-0 rounded-full px-2.5 text-[11px] text-muted-foreground"
                onClick={onBack}
              >
                <ArrowLeft className="mr-1.5 h-3.5 w-3.5" />
                {backLabel}
              </Button>
            ) : null}
            <div className="min-w-0">
              <DialogTitle className="text-[14px] font-semibold tracking-[-0.01em]">
                {title}
              </DialogTitle>
              {description ? (
                <div className="mt-1 line-clamp-2 text-[11px] leading-4.5 text-muted-foreground">
                  {description}
                </div>
              ) : null}
            </div>
          </div>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-3.5 pb-[calc(env(safe-area-inset-bottom,0px)+14px)] pt-3 touch-pan-y [-webkit-overflow-scrolling:touch]">
          {children}
        </div>
      </DialogContent>
    </Dialog>
  );
}
