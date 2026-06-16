import { Button, ButtonGroup } from "@mui/material";
import { appWindow } from "@tauri-apps/api/window";
import {
  CloseRounded,
  CropSquareRounded,
  FilterNoneRounded,
  HorizontalRuleRounded,
} from "@mui/icons-material";
import { useEffect, useState } from "react";
import LockIcon from "@/assets/icons/lock.svg?react";
import LockOpenIcon from "@/assets/icons/lock_open.svg?react";
import RestartIcon from "@/assets/icons/restart.svg?react";
import { Notice } from "@/components/base";
import { restartSidecar, setWindowSizeLocked } from "@/services/cmds";
import { useVerge } from "@/hooks/use-verge";

interface LayoutControlProps {
  nativeDecorations?: boolean;
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
    svg: {
      width: 20,
      height: 20,
      display: "block",
      transform: "scale(0.9)",
    },
    "svg path": {
      fill: "currentColor",
    },
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
      Notice.error(err?.message || err.toString());
    }
  };

  const onRestartSidecar = async () => {
    try {
      await restartSidecar();
      Notice.success("Restart clash core successfully");
    } catch (err: any) {
      Notice.error(err?.message || err.toString());
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
          color: isSizeLocked ? "error.main" : "text.secondary",
        }}
        onClick={onToggleSizeLocked}
      >
        {isSizeLocked ? <LockIcon /> : <LockOpenIcon />}
      </Button>

      <Button size="small" sx={controlButtonSx} onClick={onRestartSidecar}>
        <RestartIcon />
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
          svg: { ...controlButtonSx.svg, transform: "scale(1.05)" },
          ":hover": { bgcolor: "#ff000090" },
        }}
        onClick={() => appWindow.hide().catch(() => undefined)}
      >
        <CloseRounded fontSize="small" />
      </Button>
    </ButtonGroup>
  );
};
