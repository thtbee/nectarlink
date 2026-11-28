// SPDX-License-Identifier: GPL-3.0-or-later
import css from './receiver.css' with {type: 'text'};

const copy = {
  en: {
    upload: 'Choose a file to upload',
    download: 'Download file',
    lan: 'Local network',
    relay: 'Internet',
    waiting: 'Connecting',
    transfer: 'File transfer',
    sent: 'File sent',
    error: 'Transfer failed',
    expired: 'This link has expired.',
    size: 'The Internet route accepts files up to 128 MiB. Use the local link for larger files.',
    linkSize: 'This link accepts files up to 128 MiB.',
  },
  ru: {
    upload: 'Выбрать файл для отправки',
    download: 'Скачать файл',
    lan: 'Локальная сеть',
    relay: 'Интернет',
    waiting: 'Подключение',
    transfer: 'Передача файла',
    sent: 'Файл отправлен',
    error: 'Не удалось передать файл',
    expired: 'Срок действия ссылки истёк.',
    size: 'Через интернет можно передать файл до 128 МиБ. Для большего файла откройте локальную ссылку.',
    linkSize: 'По этой ссылке можно передать файл до 128 МиБ.',
  },
};

const icons = {
  upload: '<path d="M12 16V3m-5 5 5-5 5 5M4 16v5h16v-5"/>',
  download: '<path d="M12 3v13m-5-5 5 5 5-5M4 16v5h16v-5"/>',
  lan: '<path d="M2 8a16 16 0 0 1 20 0M5 12a11 11 0 0 1 14 0m-11 4a6 6 0 0 1 8 0m-4 4h.01"/>',
  relay: '<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18"/>',
  waiting: '<path d="M21 12a9 9 0 1 0-9 9"/>',
  sent: '<path d="m5 12 4 4L19 6"/>',
  error: '<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4h.01"/>',
};

export function createReceiverView() {
  const locale = navigator.language?.startsWith('ru') ? 'ru' : 'en';
  const strings = copy[locale];
  document.documentElement.lang = locale;
  const style = document.createElement('style');
  style.textContent = css;
  document.head.append(style);

  const actions = document.createElement('main');
  document.body.replaceChildren(actions);
  let activity, total, direction = 'download';
  function decorate(control, icon, label) {
    control.className = 'control';
    control.setAttribute('aria-label', label);
    control.innerHTML = `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${icons[icon]}</svg>`;
    return control;
  }
  function button(icon, label = strings[icon], onClick) {
    const control = decorate(document.createElement('button'), icon, label);
    control.type = 'button';
    control.disabled = !onClick;
    control.onclick = onClick;
    return control;
  }
  function show(...controls) {
    actions.replaceChildren(...controls);
  }
  function waitForChoice(signal, setup) {
    signal.throwIfAborted();
    return new Promise((resolve, reject) => {
      const abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, {once: true});
      setup((choice) => {
        signal.removeEventListener('abort', abort);
        view.waiting();
        resolve(choice);
      });
    });
  }
  const view = {
    chooseLocal(signal) {
      return waitForChoice(signal, (select) => show(
        button('lan', strings.lan, () => select(true)),
        button('relay', strings.relay, () => select(false)),
      ));
    },
    waiting() {
      activity = button('waiting');
      activity.classList.add('waiting');
      show(activity);
    },
    chooseFile(limit, relay, signal) {
      direction = 'upload';
      const input = document.createElement('input');
      input.type = 'file';
      input.hidden = true;
      return waitForChoice(signal, (select) => {
        input.onchange = () => {
          const file = input.files[0];
          if (!file) return;
          if (limit !== null && file.size > limit) {
            input.value = '';
            alert(relay ? strings.size : strings.linkSize);
            return;
          }
          select(file);
        };
        show(button('upload', strings.upload, () => input.click()), input);
      });
    },
    file(size) {
      total = size;
      activity = button(direction, strings.transfer);
      activity.classList.add('transfer');
      activity.setAttribute('role', 'progressbar');
      show(activity);
      view.progress(0);
    },
    progress(bytes) {
      const percent = total ? Math.floor(bytes / total * 100) : 100;
      activity.setAttribute('aria-valuenow', percent);
      activity.style.setProperty('--progress', `${percent}%`);
    },
    ready(link) {
      show(decorate(link, 'download', strings.download));
    },
    sent() {
      show(button('sent'));
    },
    error(message) {
      const detail = {
        'Ticket expired': strings.expired,
        'The relay file limit is 128 MiB.': strings.size,
      }[message] ?? strings.error;
      show(button('error', detail, () => alert(detail)));
      if (detail === strings.size) alert(detail);
    },
  };
  view.waiting();
  return view;
}
