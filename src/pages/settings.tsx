import { Box, Grid, IconButton } from "@mui/material";
import { useLockFn } from "ahooks";
import { useTranslation } from "react-i18next";
import { BasePage, formatNoticeMessage, Notice } from "@/components/base";
import { GitHub } from "@mui/icons-material";
import { openWebUrl } from "@/services/cmds";
import SettingVerge from "@/components/setting/setting-verge";
import SettingClash from "@/components/setting/setting-clash";
import SettingSystem from "@/components/setting/setting-system";
import { atomThemeMode } from "@/services/states";
import { useRecoilState } from "recoil";

const SettingPage = () => {
  const { t } = useTranslation();

  const onError = (err: any) => {
    Notice.error(formatNoticeMessage(err));
  };

  const toGithubRepo = useLockFn(() => {
    return openWebUrl("https://github.com/CarterLeeAlt/Clash-Verge-Buty");
  });

  const [mode] = useRecoilState(atomThemeMode);
  console.log(mode);
  const isDark = mode === "light" ? false : true;

  return (
    <BasePage
      title={t("Settings")}
      contentStyle={{
        marginTop: 3,
        marginBottom: 4,
        boxSizing: "border-box",
      }}
      header={
        <IconButton
          size="medium"
          color="inherit"
          title="@CarterLeeAlt/Clash-Verge-Buty"
          onClick={toGithubRepo}
        >
          <GitHub fontSize="inherit" />
        </IconButton>
      }
    >
      <Grid container spacing={{ xs: 1.5, lg: 1.5 }}>
        <Grid item xs={12} md={6}>
          <Box
            sx={{
              borderRadius: 2,
              marginBottom: 1.5,
              backgroundColor: isDark ? "#282a36" : "#ffffff",
            }}
          >
            <SettingSystem onError={onError} />
          </Box>
          <Box
            sx={{
              borderRadius: 2,
              backgroundColor: isDark ? "#282a36" : "#ffffff",
            }}
          >
            <SettingClash onError={onError} />
          </Box>
        </Grid>
        <Grid item xs={12} md={6}>
          <Box
            sx={{
              borderRadius: 2,
              backgroundColor: isDark ? "#282a36" : "#ffffff",
            }}
          >
            <SettingVerge onError={onError} />
          </Box>
        </Grid>
      </Grid>
    </BasePage>
  );
};

export default SettingPage;
