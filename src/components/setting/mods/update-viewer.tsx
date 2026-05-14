import { forwardRef, useImperativeHandle, useState } from "react";
import { useTranslation } from "react-i18next";
import { BaseDialog, DialogRef } from "@/components/base";

export const UpdateViewer = forwardRef<DialogRef>((props, ref) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  useImperativeHandle(ref, () => ({
    open: () => setOpen(true),
    close: () => setOpen(false),
  }));

  return (
    <BaseDialog
      open={open}
      title={t("Check for Updates")}
      okBtn={t("Ok")}
      cancelBtn={null}
      onClose={() => setOpen(false)}
      onCancel={() => setOpen(false)}
      onOk={() => setOpen(false)}
    />
  );
});
