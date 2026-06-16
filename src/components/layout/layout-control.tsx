import { Box, Button, ButtonGroup } from "@mui/material";
import { appWindow } from "@tauri-apps/api/window";
import {
  CloseRounded,
  CropSquareRounded,
  FilterNoneRounded,
  HorizontalRuleRounded,
} from "@mui/icons-material";
import { useEffect, useState } from "react";
import { Notice } from "@/components/base";
import { setWindowSizeLocked } from "@/services/cmds";
import { useVerge } from "@/hooks/use-verge";

interface LayoutControlProps {
  nativeDecorations?: boolean;
}

type ControlGlyphProps = {
  type: "lock" | "unlock" | "pin" | "pinOutlined";
};

const ControlGlyph = ({ type }: ControlGlyphProps) => {
  const isLock = type === "lock" || type === "unlock";
  const isFilled = type === "pin";

  if (isLock) {
    return (
      <Box
        component="span"
        sx={{
          position: "relative",
          display: "block",
          width: 16,
          height: 16,
          color: "currentColor",
          boxSizing: "border-box",
          "&::before": {
            content: '""',
            position: "absolute",
            left: type === "unlock" ? 7 : 4,
            top: type === "unlock" ? 0 : 1,
            width: 8,
            height: 7,
            border: "2px solid currentColor",
            borderBottom: 0,
            borderRadius: "8px 8px 0 0",
            boxSizing: "border-box",
            transform: type === "unlock" ? "rotate(-35deg)" : "none",
            transformOrigin: "left bottom",
          },
          "&::after": {
            content: '""',
            position: "absolute",
            left: 3,
            bottom: 2,
            width: 10,
            height: 8,
            border: "2px solid currentColor",
            borderRadius: "2px",
            boxSizing: "border-box",
            backgroundColor: "transparent",
          },
        }}
      />
    );
  }

  return (
    <Box
      component="span"
      sx={{
        position: "relative",
        display: "block",
        width: 16,
        height: 16,
        color: "currentColor",
        transform: "rotate(35deg)",
        boxSizing: "border-box",
        "&::before": {
          content: '""',
          position: "absolute",
          left: 5,
          top: 1,
          width: 6,
          height: 8,
          border: "2px solid currentColor",
          borderRadius: "2px 2px 1px 1px",
          backgroundColor: isFilled ? "currentColor" : "transparent",
          boxSizing: "border-box",
        },
        "&::after": {
          content: '""',
          position: "absolute",
          left: 7,
          top: 8,
          width: 2,
          height: 7,
          borderRadius: "999px",
          backgroundColor: "currentColor",
          boxSizing: "border-box",
        },
      }}
    />
  );
};

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
      <Button
        size="small"
        sx={{
          ...controlButtonSx,
          color: isSizeLocked ? "error.main" : "text.secondary",
        }}
        onClick={onToggleSizeLocked}
      >
        <ControlGlyph type={isSizeLocked ? "lock" : "unlock"} />
      </Button>

      <Button
        size="small"
        sx={controlButtonSx}
        onClick={() => {
          appWindow.setAlwaysOnTop(!isPined);
          setIsPined((isPined) => !isPined);
        }}
      >
        <ControlGlyph type={isPined ? "pin" : "pinOutlined"} />
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
