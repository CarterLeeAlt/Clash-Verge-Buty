import { useMemo, useState } from "react";
import { useRecoilState, useRecoilValue } from "recoil";
import {
  Box,
  Button,
  IconButton,
  MenuItem,
  Paper,
  Select,
  TextField,
  Alert,
} from "@mui/material";
import { Virtuoso } from "react-virtuoso";
import { useTranslation } from "react-i18next";
import {
  PlayCircleOutlineRounded,
  PauseCircleOutlineRounded,
} from "@mui/icons-material";
import { atomEnableLog, atomLogConnState, atomLogData, atomLogError } from "@/services/states";
import { BaseEmpty, BasePage } from "@/components/base";
import LogItem from "@/components/log/log-item";

const LogPage = () => {
  const { t } = useTranslation();
  const [logData, setLogData] = useRecoilState(atomLogData);
  const [enableLog, setEnableLog] = useRecoilState(atomEnableLog);
  const logError = useRecoilValue(atomLogError);
  const logConnState = useRecoilValue(atomLogConnState);

  const [logState, setLogState] = useState("all");
  const [filterText, setFilterText] = useState("");

  const filterLogs = useMemo(() => {
    return logData.filter((data) => {
      return (
        (filterText.trim() === "" ||
          data.payload.toLowerCase().includes(filterText.trim().toLowerCase())) &&
        (logState === "all" ? true : data.type.includes(logState))
      );
    });
  }, [logData, logState, filterText]);

  return (
    <BasePage
      full
      title={t("Logs")}
      contentStyle={{ height: "100%" }}
      header={
        <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
          <IconButton
            size="small"
            color="inherit"
            onClick={() => setEnableLog((e) => !e)}
          >
            {enableLog ? (
              <PauseCircleOutlineRounded />
            ) : (
              <PlayCircleOutlineRounded />
            )}
          </IconButton>

          <Button
            size="small"
            variant="contained"
            onClick={() => setLogData([])}
          >
            {t("Clear")}
          </Button>
        </Box>
      }
    >
      {enableLog && logConnState === "reconnecting" && (
        <Box sx={{ px: "10px", pb: 0.5 }}>
          <Alert severity="info" variant="outlined" sx={{ py: 0 }}>
            {logError || "日志流短暂中断，正在自动重连…"}
          </Alert>
        </Box>
      )}

      <Box
        sx={{
          pt: 1,
          mb: 0.5,
          mx: "10px",
          height: "36px",
          display: "flex",
          alignItems: "center",
        }}
      >
        <Select
          size="small"
          autoComplete="off"
          value={logState}
          onChange={(e) => setLogState(e.target.value)}
          sx={{
            width: 120,
            height: 33.375,
            mr: 1,
            '[role="button"]': { py: 0.65 },
          }}
        >
          <MenuItem value="all">ALL</MenuItem>
          <MenuItem value="inf">INFO</MenuItem>
          <MenuItem value="warn">WARN</MenuItem>
          <MenuItem value="err">ERROR</MenuItem>
        </Select>

        <TextField
          hiddenLabel
          fullWidth
          size="small"
          autoComplete="off"
          spellCheck="false"
          variant="outlined"
          placeholder={t("Filter conditions")}
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
          sx={{ input: { py: 0.65, px: 1.25 } }}
        />
      </Box>

      <Box height="calc(100% - 50px)">
        {filterLogs.length > 0 ? (
          <Virtuoso
            data={filterLogs}
            itemContent={(index, item) => <LogItem value={item} />}
            followOutput={false}
          />
        ) : (
          <BaseEmpty text={logError ?? "No Logs"} />
        )}
      </Box>
    </BasePage>
  );
};

export default LogPage;
