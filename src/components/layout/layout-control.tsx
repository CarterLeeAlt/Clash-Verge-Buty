import { Button, ButtonGroup } from "@mui/material";
import { appWindow } from "@tauri-apps/api/window";
import {
  CloseRounded,
  CropSquareRounded,
  FilterNoneRounded,
  HorizontalRuleRounded,
  LockOpenRounded,
  LockRounded,
  PushPinOutlined,
  PushPinRounded,
} from "@mui/icons-material";
import LockRounded from "@mui/icons-material/LockRounded";
import LockOpenRounded from "@mui/icons-material/LockOpenRounded";
import { useEffect, useState } from "react";
import { Notice } from "@/components/base";
import { setWindowSizeLocked } from "@/services/cmds";
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
    svg: { transform: "scale(0.9)" },
  };

  const { verge, mutateVerge } = useVerge();
  const [isMaximized, setIsMaximized] = useState(false);
  const [isPined, setIsPined] = useState(false);
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
      <Button size="small" sx={controlButtonSx} onClick={onToggleSizeLocked}>
        {isSizeLocked ? (
          <LockRounded
            fontSize="small"
            sx={{ color: "error.main", display: "block" }}
          />
        ) : (
          <LockOpenRounded
            fontSize="small"
            sx={{ color: "text.secondary", display: "block" }}
          />
        )}
      </Button>

      <Button
        size="small"
        sx={controlButtonSx}
        onClick={() => {
          appWindow.setAlwaysOnTop(!isPined);
          setIsPined((isPined) => !isPined);
        }}
      >
        {isPined ? (
          <PushPinRounded fontSize="small" />
        ) : (
          <PushPinOutlined fontSize="small" />
        )}
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
