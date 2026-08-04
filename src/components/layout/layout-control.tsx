import { Button, ButtonGroup } from "@mui/material";
import { appWindow } from "@tauri-apps/api/window";
import {
  CloseRounded,
  CropSquareRounded,
  FilterNoneRounded,
  HorizontalRuleRounded,
} from "@mui/icons-material";
import { useEffect, useState } from "react";
import lockIconUrl from "@/assets/icons/lock.svg";
import lockOpenIconUrl from "@/assets/icons/lock_open.svg";
import restartIconUrl from "@/assets/icons/restart.svg";
import { formatNoticeMessage, Notice } from "@/components/base";
import { restartSidecar, setWindowSizeLocked } from "@/services/cmds";
import { useVerge } from "@/hooks/use-verge";

interface LayoutControlProps {
  nativeDecorations?: boolean;
}

function LocalSvgIcon(props: { src: string; size?: number }) {
  const { src, size = 20 } = props;

  return (
    <img
      aria-hidden
      src={src}
      draggable={false}
      style={{
        display: "block",
        width: size,
        height: size,
        objectFit: "contain",
        pointerEvents: "none",
        userSelect: "none",
      }}
    />
  );
}

export const LayoutControl = ({
  nativeDecorations = false,
}: LayoutControlProps) => {
  const minWidth = 40;
  const controlButtonSx = {
    minWidth,
    height: "100%",
    p: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    color: "text.secondary",
  };

  const { verge, mutateVerge } = useVerge();
  const [isMaximized, setIsMaximized] = useState(false);
  const [isSizeLocked, setIsSizeLocked] = useState(false);

  useEffect(() => {
    if (nativeDecorations) return;

    appWindow
      .isMaximized()
      .then(setIsMaximized)
      .catch(() => undefined);
  }, [nativeDecorations]);

  useEffect(() => {
    setIsSizeLocked(verge?.window_size_locked ?? false);
  }, [verge?.window_size_locked]);

  const onToggleSizeLocked = async () => {
    const nextLocked = !isSizeLocked;

    try {
      await setWindowSizeLocked(nextLocked);
      setIsSizeLocked(nextLocked);
      if (nextLocked) setIsMaximized(false);
      await mutateVerge();
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  };

  const onRestartSidecar = async () => {
    try {
      await restartSidecar();
      Notice.success("Mihomo core restarted.");
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  };

  const onToggleMaximize = () => {
    if (isSizeLocked) return;

    setIsMaximized((isMaximized) => !isMaximized);
    appWindow.toggleMaximize();
  };

  if (nativeDecorations) return null;

  return (
    <ButtonGroup
      variant="text"
      sx={{
        height: "100%",
        ".MuiButtonGroup-grouped": {
          borderRadius: "0px",
          borderRight: "0px",
          height: "100%",
          p: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        },
      }}
    >
      <Button
        size="small"
        sx={{
          ...controlButtonSx,
          opacity: isSizeLocked ? 1 : 0.72,
        }}
        onClick={onToggleSizeLocked}
      >
        <LocalSvgIcon src={isSizeLocked ? lockIconUrl : lockOpenIconUrl} />
      </Button>

      <Button size="small" sx={controlButtonSx} onClick={onRestartSidecar}>
        <LocalSvgIcon src={restartIconUrl} />
      </Button>

      <Button
        size="small"
        sx={controlButtonSx}
        onClick={() => appWindow.minimize()}
      >
        <HorizontalRuleRounded fontSize="small" />
      </Button>

      <Button
        size="small"
        disabled={isSizeLocked}
        sx={controlButtonSx}
        onClick={onToggleMaximize}
      >
        {isMaximized ? (
          <FilterNoneRounded
            fontSize="small"
            style={{
              transform: "rotate(180deg) scale(0.7)",
            }}
          />
        ) : (
          <CropSquareRounded fontSize="small" />
        )}
      </Button>

      <Button
        size="small"
        sx={{
          ...controlButtonSx,
          svg: { transform: "scale(1.05)" },
          ":hover": { bgcolor: "#ff000090" },
        }}
        onClick={() => appWindow.hide().catch(() => undefined)}
      >
        <CloseRounded fontSize="small" />
      </Button>
    </ButtonGroup>
  );
};
