---
layout: home
title: "Raccoin – Crypto Tax Tool"
list_title: Blog
image: /screenshots/raccoin-welcome.png
---

<img src="/raccoin_ui/ui/icons/app-icon.svg" width="150" height="150" style="margin: 10px auto; display: block;">

# Raccoin – Crypto Tax Tool

Raccoin makes it easy to see the current state and the history of your crypto
portfolio and to generate relevant reports for declaring capital gain income
tax. It currently works in Euro using the [FIFO
method](https://en.wikipedia.org/wiki/FIFO_and_LIFO_accounting), but since it is
[open source](https://github.com/bjorn/raccoin) anyone can extend it to suit
their needs.

<div class="thumbnails">
<img class="thumbnail" src="/screenshots/raccoin-welcome.png" alt="Welcome screen">
<img class="thumbnail" src="/screenshots/raccoin-wallets.png" alt="The wallets page shows the transaction sources">
<img class="thumbnail" src="/screenshots/raccoin-transactions.png" alt="The transactions page provides a detailed view of events">
<img class="thumbnail" src="/screenshots/raccoin-reports.png" alt="Reports can be exported as CSV files">
</div>

<div id="fullpage" onclick="this.style.display='none';">
    <img id="fullpage-image">
    <div id="fullpage-caption"></div>
</div>

<script>
const thumbnails = document.querySelectorAll('.thumbnail');
const fullPage = document.querySelector('#fullpage');
const fullPageImg = document.querySelector('#fullpage-image');
const fullPageCaption = document.querySelector('#fullpage-caption');

thumbnails.forEach(thumbnail => {
  thumbnail.addEventListener('click', function() {
    fullPageImg.src = thumbnail.src;
    fullPageCaption.innerHTML = thumbnail.alt;
    fullPage.style.display = 'flex';
  });
});
</script>
