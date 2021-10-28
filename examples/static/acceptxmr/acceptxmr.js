function start() {
    var message = document.getElementById("acceptxmr-message").value;

    // Hide prep stuff, show payment stuff.
    document.getElementById("acceptxmr-instruction").innerHTML = "Loading...";
    document.getElementById("acceptxmr-preperation-content").style.display = "None";
    document.getElementById("acceptxmr-payment-content").style.display = "inherit"

    // Start websocket.
    let socket = new WebSocket("ws://localhost:8080/ws/");

    // Send message.
    socket.onopen = function (event) {
        socket.send(message);
    }

    socket.onmessage = function (event) {
        var invoiceUpdate = JSON.parse(event.data);

        // Show paid/due.
        document.getElementById("acceptxmr-paid").innerHTML = picoToXMR(invoiceUpdate.amount_paid);
        document.getElementById("acceptxmr-due").innerHTML = picoToXMR(invoiceUpdate.amount_requested);

        // Show confirmations/required.
        var confirmations = Math.max(0, invoiceUpdate.confirmations);
        document.getElementById("acceptxmr-confirmations").innerHTML = confirmations;
        document.getElementById("acceptxmr-confirmations-required").innerHTML = invoiceUpdate.confirmations_required;

        // Show instructive text depending on invoice state.
        var instructionString = "Loading...";
        var instructionClass = "acceptxmr-instruction";
        var newAddressBtnHidden = true;
        if (confirmations >= invoiceUpdate.confirmations_required) {
            instructionString = "Paid! Thank you"
            socket.close();
        } else if (invoiceUpdate.amount_paid > invoiceUpdate.amount_requested) {
            instructionString = "Paid! Waiting for Confirmation..."
        } else if (invoiceUpdate.expiration_in > 2) {
            instructionString = "Send Monero to Address Below"
        } else if (invoiceUpdate.expiration_in > 0) {
            instructionString = "Address Expiring Soon";
            instructionClass += " acceptxmr-warning";
            newAddressBtnHidden = false;
        } else {
            instructionString = "Address Expired!";
            newAddressBtnHidden = false;
            socket.close();
        }
        document.getElementById("acceptxmr-instruction").innerHTML = instructionString;
        document.getElementById("acceptxmr-instruction").classList = instructionClass;

        // Hide address if nearing expiration.
        document.getElementById("acceptxmr-new-address-btn").hidden = newAddressBtnHidden;
        document.getElementById("acceptxmr-address-copy-btn").disabled = !newAddressBtnHidden;
        if (newAddressBtnHidden) {
            var address = invoiceUpdate.address;
            document.getElementById("acceptxmr-address").innerHTML = address;

            var qr = qrcode(0, "M");
            qr.addData(address);
            qr.make();
            document.getElementById('acceptxmr-qrcode-container').innerHTML = qr.createSvgTag({ scalable: true });
        } else {
            document.getElementById("acceptxmr-address").innerHTML = "Expiring or expired...";
            document.getElementById('acceptxmr-qrcode-container').innerHTML = "<svg viewBox=\"0 0 100 100\" src=\"\"></svg>";
        }

    };

    // If the websocket closes cleanly, log it. Otherwise, alert the user.
    socket.onclose = function (event) {
        if (event.code === 1000) {
            console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
        } else {
            // Server process killed or network down.
            // Event.code is usually 1006 in this case.
            alert('Connection died. If you have paid already, rest assured that it will still be processed.');
        }
    };

    socket.onerror = function (error) {
        alert(`[error] ${error.message}`);
    };
}

// Convert from piconeros to monero.
function picoToXMR(amount) {
    let divisor = 1_000_000_000_000;
    let xmr = Math.floor(amount / divisor) + amount % divisor / divisor;
    return new Intl.NumberFormat(undefined, { maximumSignificantDigits: 20 }).format(xmr);
}

// Make the copy button work.
function copyInvoiceAddress() {
    // Get the text field
    var copyText = document.getElementById("acceptxmr-address");

    // Copy the text inside the text field
    navigator.clipboard.writeText(copyText.innerHTML);

    // Provide feedback
    document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copied!";
    setTimeout(function () {
        document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copy";
    }, 1000);
}
