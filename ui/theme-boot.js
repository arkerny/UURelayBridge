/* 首帧主题：在 CSS 之前执行，避免"先白后黑"的闪烁。
 * 这里只按系统偏好给一个初值；真正的设置（跟随系统/浅色/深色）在 app.js
 * 拿到 bootstrap 之后再覆盖。 */
(function () {
  try {
    var dark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
  } catch (e) {
    document.documentElement.setAttribute("data-theme", "dark");
  }
})();
