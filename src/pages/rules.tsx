import useSWR from "swr";
import { useState, useMemo, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Box, TextField } from "@mui/material";
import { getRules } from "@/services/api";
import { BaseEmpty, BasePage } from "@/components/base";
import RuleItem from "@/components/rule/rule-item";
import { ProviderButton } from "@/components/rule/provider-button";

const RulesPage = () => {
  const { t } = useTranslation();
  const { data } = useSWR("getRules", getRules);

  const [filterText, setFilterText] = useState("");
  const virtuosoRef = useRef<VirtuosoHandle>(null);

  const rules = useMemo(() => {
    const ruleList = Array.isArray(data) ? data : [];
    return ruleList.filter((each) => each.payload.includes(filterText));
  }, [data, filterText]);

  useEffect(() => {
    if (rules.length > 0) {
      virtuosoRef.current?.scrollToIndex({
        index: 0,
        align: "start",
        behavior: "auto",
      });
    }
  }, [rules, filterText]);

  return (
    <BasePage
      full
      title={t("Rules")}
      contentStyle={{ height: "100%" }}
      header={
        <Box display="flex" alignItems="center" gap={1}>
          <ProviderButton />
        </Box>
      }
    >
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
        <TextField
          hiddenLabel
          fullWidth
          size="small"
          autoComplete="off"
          variant="outlined"
          spellCheck="false"
          placeholder={t("Filter conditions")}
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
          sx={{ input: { py: 0.65, px: 1.25 } }}
        />
      </Box>

      <Box height="calc(100% - 50px)">
        {rules.length > 0 ? (
          <Virtuoso
            ref={virtuosoRef}
            data={rules}
            itemContent={(index, item) => (
              <RuleItem index={index + 1} value={item} />
            )}
          />
        ) : (
          <BaseEmpty text="No Rules" />
        )}
      </Box>
    </BasePage>
  );
};

export default RulesPage;
