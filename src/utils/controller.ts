const SCHEME_RE = /^(https?|wss?):\/\//i;

export const normalizeControllerHost = (server: string): string => {
  if (!server) return "";

  let value = server.trim();
  if (!value) return "";

  value = value.replace(SCHEME_RE, "");
  value = value.replace(/\/.*$/, "");

  if (/^\d+$/.test(value)) return `127.0.0.1:${value}`;
  if (/^:\d+$/.test(value)) return `127.0.0.1${value}`;

  const match = value.match(/^\[([^\]]+)\](?::(\d+))?$/);
  if (match) {
    const host = match[1];
    const port = match[2];
    if (host === "::" || host === "0:0:0:0:0:0:0:0") {
      return port ? `127.0.0.1:${port}` : "127.0.0.1";
    }
    return value;
  }

  const [host, port] = value.split(":");
  if (host === "0.0.0.0") return port ? `127.0.0.1:${port}` : "127.0.0.1";
  if (host === "::" || value === "::") return port ? `127.0.0.1:${port}` : "127.0.0.1";

  return value;
};

export const buildControllerWsUrl = (
  server: string,
  path: string,
  token?: string,
): string => {
  const host = normalizeControllerHost(server);
  if (!host) return "";

  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  const encodedToken = token ? `?token=${encodeURIComponent(token)}` : "";
  return `ws://${host}${normalizedPath}${encodedToken}`;
};
