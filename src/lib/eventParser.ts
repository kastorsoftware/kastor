/**
 * Helpers for parsing backend log events that can arrive in RU or EN.
 * Backend emits translated strings, so we need to match both languages.
 */

/** Check if event signals task completion */
export function isDone(msg: string): boolean {
  return msg === "Завершено" || msg === "Done";
}

/** Check if event contains an error */
export function isError(msg: string): boolean {
  return msg.includes("ОШИБКА") || msg.includes("ERROR") || msg.includes("ошибка:") || msg.includes("error:");
}

/** Check if event signals a successful invite */
export function isInvited(msg: string): boolean {
  return msg.includes("приглашён") || msg.includes("добавлен") || msg.includes("invited:") || msg.includes("added:");
}

/** Check if event signals a successful comment */
export function isCommentSent(msg: string): boolean {
  return msg.includes("комментарий отправлен") || msg.includes("comment sent");
}

/** Check if event signals a reply sent */
export function isReplySent(msg: string): boolean {
  return msg.includes("ответ отправлен") || msg.includes("голосовое отправлено") || msg.includes("reply sent") || msg.includes("voice sent");
}

/** Check if event signals a skipped message */
export function isSkipped(msg: string): boolean {
  return msg.includes("пропущено") || msg.includes("бан-слово") || msg.includes("skipped") || msg.includes("ban word");
}

/** Check if event signals a successful boost action */
export function isBoostDone(msg: string): boolean {
  return msg.includes("выполнено") || msg.includes("успешно") || msg.includes("done:") || msg.includes("viewed") || msg.includes("просмотрено");
}

/** Check if event signals a report sent */
export function isReportSent(msg: string): boolean {
  return msg.includes("репорт отправлен") || msg.includes("report sent") || msg.includes("reported");
}

/** Check if event signals a bot created */
export function isBotCreated(msg: string): boolean {
  return msg.includes("бот создан") || msg.includes("токен:") || msg.includes("bot created") || msg.includes("token:");
}

/** Check if event signals channel created */
export function isChannelCreated(msg: string): boolean {
  return msg.includes("создан") || msg.includes("created");
}

/** Check if event signals a copied post (cloner) */
export function isCopied(msg: string): boolean {
  return msg.includes("скопировано") || msg.includes("copied");
}

/** Check if event signals a skipped post (cloner) */
export function isClonerSkipped(msg: string): boolean {
  return msg.includes("пропущено:") || msg.includes("пропущен") || msg.includes("skipped:");
}

/** Check if event signals a new search result */
export function isSearchFound(msg: string): boolean {
  return msg.includes("новых:") || msg.includes("new:");
}

/** Extract count from search result */
export function extractSearchCount(msg: string): number | null {
  const m = msg.match(/(?:новых|new):\s*(\d+)/);
  return m ? Number(m[1]) : null;
}

/** Check if event signals masslooking view */
export function isMasslookingViewed(msg: string): boolean {
  return msg.includes("просмотрено") || msg.includes("viewed");
}

/** Check if event signals masslooking reaction */
export function isMasslookingReacted(msg: string): boolean {
  return msg.includes("реакция") || msg.includes("reacted");
}

/** Check if event signals masslooking reply */
export function isMasslookingReplied(msg: string): boolean {
  return msg.includes("ответ отправлен") || msg.includes("replied");
}
