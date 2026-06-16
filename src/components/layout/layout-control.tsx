import { Button, ButtonGroup, SvgIcon } from "@mui/material";
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

type ControlIconProps = {
  type: "lock" | "unlock" | "pin" | "pinOutlined";
};

const ControlIcon = ({ type }: ControlIconProps) => {
  switch (type) {
    case "lock":
      return (
        <SvgIcon fontSize="small" viewBox="0 0 24 24">
          <path d="M17 8h-1V6c0-2.76-2.24-5-5-5S6 3.24 6 6v2H5c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2ZM8 6c0-1.66 1.34-3 3-3s3 1.34 3 3v2H8V6Zm9 14H5V10h12v10Z" />
        </SvgIcon>
      );
    case "unlock":
      return (
        <SvgIcon fontSize="small" viewBox="0 0 24 24">
          <path d="M17 8H9V6c0-1.66 1.34-3 3-3 1.12 0 2.09.61 2.61 1.52.28.48.89.64 1.37.36.48-.28.64-.89.36-1.37C15.47 1.99 13.85 1 12 1 9.24 1 7 3.24 7 6v2H5c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2Zm0 12H5V10h12v10Z" />
        </SvgIcon>
      );
    case "pin":
      return (
        <SvgIcon fontSize="small" viewBox="0 0 24 24">
          <path d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3Z" />
        </SvgIcon>
      );
    case "pinOutlined":
      return (
        <SvgIcon fontSize="small" viewBox="0 0 24 24">
          <path d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3Zm-8.17 3C9.17 11.01 10 9.43 10 7.7V4h4v3.7c0 1.73.83 3.31 2.17 4.3H7.83Z" />
        </SvgIcon>
      );
  }
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
        <ControlIcon type={isSizeLocked ? "lock" : "unlock"} />
      </Button>

      <Button
        size="small"
        sx={controlButtonSx}
        onClick={() => {
          appWindow.setAlwaysOnTop(!isPined);
          setIsPined((isPined) => !isPined);
        }}
      >
        <ControlIcon type={isPined ? "pin" : "pinOutlined"} />
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
