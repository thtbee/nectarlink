import css from './receiver.css' with {type: 'text'};

const copy = {
  en: {
    title: 'Receive a file',
    connecting: 'Connecting to the sender…',
    waiting: 'Getting file details…',
    receiving: 'Receiving and verifying the file…',
    ready: 'Your file is ready to save.',
    download: 'Download file',
    hold: 'Keep the sender app open until the transfer finishes.',
    private: 'End-to-end encrypted transfer',
    expired: 'This link has expired.',
    used: 'This link has expired or has already been used.',
    unreachable: 'Could not reach the sender.',
    corrupt: 'The transfer could not be verified.',
    invalid: 'This AnySend link is incomplete or invalid.',
    failed: 'The transfer could not be completed.',
    newTicket: 'Ask the sender to generate a fresh QR code.',
    progress: 'File transfer progress',
    sendTitle: 'Send a file',
    choose: 'Choose a file to send to the PC.',
    tooLarge: 'Choose a file up to 128 MiB.',
    sending: 'Sending the file…',
    sent: 'The file was saved on the PC.',
    units: ['B', 'KiB', 'MiB'],
  },
  ru: {
    title: 'Получить файл',
    connecting: 'Подключаемся к отправителю…',
    waiting: 'Получаем сведения о файле…',
    receiving: 'Получаем и проверяем файл…',
    ready: 'Файл готов к сохранению.',
    download: 'Скачать файл',
    hold: 'Оставьте приложение отправителя открытым до конца передачи.',
    private: 'Передача со сквозным шифрованием',
    expired: 'Срок действия ссылки истёк.',
    used: 'Ссылка уже использована или срок её действия истёк.',
    unreachable: 'Не удалось связаться с отправителем.',
    corrupt: 'Не удалось проверить целостность передачи.',
    invalid: 'Ссылка AnySend неполная или повреждена.',
    failed: 'Не удалось завершить передачу.',
    newTicket: 'Попросите отправителя создать новый QR-код.',
    progress: 'Ход передачи файла',
    sendTitle: 'Отправить файл',
    choose: 'Выберите файл для отправки на ПК.',
    tooLarge: 'Выберите файл размером до 128 МиБ.',
    sending: 'Отправляем файл…',
    sent: 'Файл сохранён на ПК.',
    units: ['Б', 'КиБ', 'МиБ'],
  },
};

export function createReceiverView() {
  const locale = navigator.language?.startsWith('ru') ? 'ru' : 'en';
  const strings = copy[locale];
  const number = new Intl.NumberFormat(locale, {maximumFractionDigits: 1});
  document.documentElement.lang = locale;
  document.title = `${strings.title} · AnySend`;
  const viewport = document.createElement('meta');
  viewport.name = 'viewport';
  viewport.content = 'width=device-width, initial-scale=1';
  const style = document.createElement('style');
  style.textContent = css;
  document.head.append(viewport, style);

  document.body.innerHTML = `
    <main class="shell">
      <header class="brand"><span class="mark" aria-hidden="true">↓</span><span>AnySend</span></header>
      <section class="card" aria-labelledby="title">
        <h1 id="title">${strings.title}</h1>
        <p id="status" role="status" aria-live="polite">${strings.connecting}</p>
        <div class="file" id="file" hidden>
          <span class="file-icon" aria-hidden="true"><i></i></span>
          <div class="file-detail"><strong id="name" dir="auto"></strong><span id="size"></span></div>
        </div>
        <div class="transfer" id="transfer">
          <progress id="progress" max="100" aria-label="${strings.progress}"></progress>
          <div class="progress-detail" id="progress-detail" hidden aria-hidden="true">
            <span id="bytes"></span><span id="percent"></span>
          </div>
        </div>
        <div id="actions" hidden></div>
        <div id="hint" class="hint">${strings.hold}</div>
      </section>
      <footer><span class="lock" aria-hidden="true"></span>${strings.private}</footer>
    </main>`;

  const get = (id) => document.getElementById(id);
  const status = get('status'), file = get('file'), transfer = get('transfer');
  const progress = get('progress'), details = get('progress-detail'), hint = get('hint');
  const actions = get('actions');
  let uploading = false;
  let total = 0;
  function formatBytes(bytes) {
    const unit = bytes < 1024 ? 0 : bytes < 1024 ** 2 ? 1 : 2;
    return `${number.format(bytes / 1024 ** unit)} ${strings.units[unit]}`;
  }
  function update(received) {
    const percent = total ? Math.floor(received / total * 100) : 100;
    progress.value = percent;
    get('bytes').textContent = `${formatBytes(received)} / ${formatBytes(total)}`;
    get('percent').textContent = `${percent}%`;
  }
  return {
    waiting() {
      status.textContent = strings.waiting;
    },
    chooseFile(limit) {
      uploading = true;
      get('title').textContent = strings.sendTitle;
      document.title = `${strings.sendTitle} · AnySend`;
      status.textContent = strings.choose;
      transfer.hidden = true;
      const input = document.createElement('input');
      input.type = 'file';
      input.setAttribute('aria-label', strings.choose);
      actions.replaceChildren(input);
      actions.hidden = false;
      return new Promise((resolve) => {
        input.onchange = () => {
          const file = input.files[0];
          if (!file) return;
          if (file.size > limit) {
            status.textContent = strings.tooLarge;
            input.value = '';
            return;
          }
          actions.hidden = true;
          actions.replaceChildren();
          resolve(file);
        };
      });
    },
    file(name, size) {
      total = size;
      get('name').textContent = name;
      get('size').textContent = formatBytes(size);
      file.hidden = false;
      transfer.hidden = false;
      details.hidden = false;
      status.textContent = uploading ? strings.sending : strings.receiving;
      update(0);
    },
    progress: update,
    ready(link) {
      update(total);
      status.textContent = strings.ready;
      status.dataset.result = 'ok';
      hint.hidden = true;
      transfer.hidden = true;
      link.textContent = strings.download;
      actions.append(link);
      actions.hidden = false;
    },
    sent() {
      status.textContent = strings.sent;
      status.dataset.result = 'ok';
      hint.hidden = true;
      transfer.hidden = true;
    },
    error(message) {
      const label = {
        'Ticket expired': 'expired',
        'Ticket used or expired': 'used',
        'Invalid ticket': 'invalid',
        'No reply': 'unreachable',
        'Relay offline': 'unreachable',
        'Relay timeout': 'unreachable',
        'Relay denied': 'unreachable',
        'Client digest mismatch': 'corrupt',
        'File authentication failed': 'corrupt',
      }[message] ?? 'failed';
      status.textContent = strings[label];
      status.dataset.result = 'error';
      transfer.hidden = true;
      hint.textContent = strings.newTicket;
    },
  };
}
