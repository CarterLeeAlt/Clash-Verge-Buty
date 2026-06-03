import { Button, ButtonGroup } from "@mui/material";
import { appWindow } from "@tauri-apps/api/window";
import {
  CloseRounded,
  CropSquareRounded,
  FilterNoneRounded,
  HorizontalRuleRounded,
  PushPinOutlined,
  PushPinRounded,
} from "@mui/icons-material";
import { useEffect, useState } from "react";

interface LayoutControlProps {
  nativeDecorations?: boolean;
}

export const LayoutControl = ({
  nativeDecorations = false,
}: LayoutControlProps) => {
  const minWidth = 40;

  const [isMaximized, setIsMaximized] = useState(false);
  const [isPined, setIsPined] = useState(false);
  useEffect(() => {
    if (nativeDecorations) return;

    appWindow
      .isMaximized()
      .then(setIsMaximized)
      .catch(() => undefined);
  }, [nativeDecorations]);

  if (nativeDecorations) return null;

  return (
    <ButtonGroup
      variant="text"
      sx={{
        height: "100%",
        ".MuiButtonGroup-grouped": {
          borderRadius: "0px",
          borderRight: "0px",
        },
      }}
    >
      <Button
        size="small"
        sx={{ minWidth, svg: { transform: "scale(0.9)" } }}
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
        sx={{ minWidth, svg: { transform: "scale(0.9)" } }}
        onClick={() => appWindow.minimize()}
      >
        <HorizontalRuleRounded fontSize="small" />
      </Button>

      <Button
        size="small"
        sx={{ minWidth, svg: { transform: "scale(0.9)" } }}
        onClick={() => {
          setIsMaximized((isMaximized) => !isMaximized);
          appWindow.toggleMaximize();
        }}
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
          minWidth,
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
