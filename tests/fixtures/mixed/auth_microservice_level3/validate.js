const crypto = require('crypto');

function validateInput(user, pass) {
  return user.length > 0 && pass.length > 0;
}

module.exports = { validateInput };
