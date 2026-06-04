import { useEffect, useSyncExternalStore } from "react";
import { Alert, Box, IconButton } from "@mui/material";
import { CloseRounded } from "@mui/icons-material";
import {
  clearNotices,
  getSnapshotNotices,
  hideNotice,
  subscribeNotices,
} from "@/services/notice-service";

export {
  DEFAULT_NOTICE_DURATION,
  hideNotice,
  Notice,
} from "@/services/notice-service";
export type { NoticeItem, NoticeType } from "@/services/notice-service";

export const NoticeManager = () => {
  const currentNotices = useSyncExternalStore(
    subscribeNotices,
    getSnapshotNotices,
    getSnapshotNotices
  );

  useEffect(() => {
    return () => {
      clearNotices();
    };
  }, []);

  return (
    <Box
      sx={{
        position: "fixed",
        top: "60px",
        right: "20px",
        zIndex: 1500,
        display: "flex",
        flexDirection: "column",
        gap: "10px",
        width: "360px",
        maxWidth: "calc(100vw - 40px)",
        pointerEvents: "none",
      }}
    >
      {currentNotices.map((notice) => (
        <Alert
          key={notice.id}
          severity={notice.type}
          variant="filled"
          sx={{
            width: "100%",
            pointerEvents: "auto",
            wordBreak: "break-word",
            color: "#fff",
            "& .MuiAlert-icon": {
              color: "#fff",
            },
            "& .MuiAlert-action": {
              color: "#fff",
            },
          }}
          action={
            <IconButton
              size="small"
              color="inherit"
              onClick={() => hideNotice(notice.id)}
            >
              <CloseRounded fontSize="inherit" />
            </IconButton>
          }
        >
          {notice.message}
        </Alert>
      ))}
    </Box>
  );
};
