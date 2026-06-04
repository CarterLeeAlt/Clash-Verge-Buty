import type { ReactNode } from "react";

export type NoticeType = "success" | "error" | "info";

export const DEFAULT_NOTICE_DURATION = 2000;

export interface NoticeItem {
  id: number;
  type: NoticeType;
  message: ReactNode;
  duration: number;
  timerId: ReturnType<typeof setTimeout>;
}

export interface ShowNoticeOptions {
  type: NoticeType;
  message: ReactNode;
  duration?: number;
}

type NoticeSubscriber = () => void;
type NoticeShortcut = (
  message: ReactNode,
  duration?: number,
  isDark?: boolean
) => number;

export interface NoticeInstance {
  (options: ShowNoticeOptions): number;
  success: NoticeShortcut;
  error: NoticeShortcut;
  info: NoticeShortcut;
}

let nextNoticeId = 0;
let notices: NoticeItem[] = [];
const subscribers = new Set<NoticeSubscriber>();

const notifySubscribers = () => {
  subscribers.forEach((subscriber) => subscriber());
};

export const getSnapshotNotices = () => notices;

export const subscribeNotices = (subscriber: NoticeSubscriber) => {
  subscribers.add(subscriber);
  return () => {
    subscribers.delete(subscriber);
  };
};

const normalizeNoticeType = (type: NoticeType): NoticeType => {
  if (type === "success" || type === "error" || type === "info") {
    return type;
  }
  return "info";
};

export const hideNotice = (id: number) => {
  const target = notices.find((notice) => notice.id === id);
  if (!target) return;

  clearTimeout(target.timerId);
  notices = notices.filter((notice) => notice.id !== id);
  notifySubscribers();
};

export const clearNotices = () => {
  notices.forEach((notice) => clearTimeout(notice.timerId));
  notices = [];
  notifySubscribers();
};

export const showNotice = ((options: ShowNoticeOptions) => {
  const id = nextNoticeId++;
  const type = normalizeNoticeType(options.type);
  const duration = DEFAULT_NOTICE_DURATION;
  const timerId = setTimeout(() => {
    hideNotice(id);
  }, duration);

  notices = [
    ...notices,
    {
      id,
      type,
      message: options.message,
      duration,
      timerId,
    },
  ];
  notifySubscribers();

  return id;
}) as NoticeInstance;

showNotice.success = (message) => showNotice({ type: "success", message });
showNotice.error = (message) => showNotice({ type: "error", message });
showNotice.info = (message) => showNotice({ type: "info", message });

export const Notice = showNotice;
